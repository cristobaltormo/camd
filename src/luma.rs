use crate::imaging::{Image, MAX_DIMENSION};

const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

const LOOKUP_BITS: u32 = 9;

#[derive(Clone)]
struct Huffman {
    lookup: Vec<u16>,
    max_code: [i32; 18],
    offset: [i32; 17],
    symbols: Vec<u8>,
}

impl Huffman {
    fn new(counts: &[u8; 16], symbols: &[u8]) -> Option<Huffman> {
        let mut lookup = vec![0u16; 1 << LOOKUP_BITS];
        let mut max_code = [-1i32; 18];
        let mut offset = [0i32; 17];
        let mut code = 0i32;
        let mut index = 0usize;
        for length in 1..=16usize {
            let count = counts[length - 1] as usize;
            offset[length] = index as i32 - code;
            for _ in 0..count {
                if code >= 1 << length {
                    return None;
                }
                if length as u32 <= LOOKUP_BITS {
                    let shift = LOOKUP_BITS - length as u32;
                    let first = (code as usize) << shift;
                    for slot in &mut lookup[first..first + (1 << shift)] {
                        *slot = ((length as u16) << 8) | symbols[index] as u16;
                    }
                }
                code += 1;
                index += 1;
            }
            max_code[length] = if count > 0 { code - 1 } else { -1 };
            code <<= 1;
        }
        max_code[17] = i32::MAX;
        Some(Huffman {
            lookup,
            max_code,
            offset,
            symbols: symbols.to_vec(),
        })
    }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buffer: u64,
    count: u32,
    marker: bool,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8], pos: usize) -> Bits<'a> {
        Bits {
            data,
            pos,
            buffer: 0,
            count: 0,
            marker: false,
        }
    }

    #[inline]
    fn refill(&mut self) {
        if !self.marker
            && let Some(chunk) = self.data.get(self.pos..self.pos + 8)
        {
            let word = u64::from_be_bytes(chunk.try_into().unwrap_or([0xFF; 8]));
            let has_ff = (!word).wrapping_sub(0x0101_0101_0101_0101) & word & 0x8080_8080_8080_8080;
            if has_ff == 0 {
                let bytes = ((64 - self.count) / 8) as usize;
                if bytes == 0 {
                    return;
                }
                let fresh = word >> (64 - bytes * 8);
                self.buffer |= fresh << (64 - self.count - bytes as u32 * 8);
                self.count += bytes as u32 * 8;
                self.pos += bytes;
                return;
            }
        }
        while self.count <= 56 {
            let mut byte = 0u8;
            if !self.marker && self.pos < self.data.len() {
                byte = self.data[self.pos];
                if byte == 0xFF {
                    match self.data.get(self.pos + 1) {
                        Some(0x00) => self.pos += 2,
                        _ => {
                            self.marker = true;
                            byte = 0;
                        }
                    }
                } else {
                    self.pos += 1;
                }
            }
            self.buffer |= (byte as u64) << (56 - self.count);
            self.count += 8;
        }
    }

    #[inline]
    fn peek(&mut self, n: u32) -> u32 {
        if self.count < n {
            self.refill();
        }
        (self.buffer >> (64 - n)) as u32
    }

    #[inline]
    fn skip(&mut self, n: u32) {
        self.buffer <<= n;
        self.count -= n;
    }

    #[inline]
    fn take(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let value = self.peek(n);
        self.skip(n);
        value
    }

    #[inline]
    fn decode(&mut self, table: &Huffman) -> Option<u8> {
        let head = self.peek(16);
        let entry = table.lookup[(head >> (16 - LOOKUP_BITS)) as usize];
        if entry != 0 {
            self.skip((entry >> 8) as u32);
            return Some(entry as u8);
        }
        let mut length = LOOKUP_BITS as usize + 1;
        while length <= 16 {
            let code = (head >> (16 - length)) as i32;
            if code <= table.max_code[length] {
                self.skip(length as u32);
                let index = (code + table.offset[length]) as usize;
                return table.symbols.get(index).copied();
            }
            length += 1;
        }
        None
    }

    #[inline]
    fn receive_extend(&mut self, size: u32) -> i32 {
        if size == 0 {
            return 0;
        }
        if size > 16 {
            return 0;
        }
        let value = self.take(size) as i32;
        if value < 1 << (size - 1) {
            value - (1 << size) + 1
        } else {
            value
        }
    }

    fn restart(&mut self) -> bool {
        self.buffer = 0;
        self.count = 0;
        self.marker = false;
        while self.pos + 1 < self.data.len() {
            if self.data[self.pos] == 0xFF && (0xD0..=0xD7).contains(&self.data[self.pos + 1]) {
                self.pos += 2;
                return true;
            }
            self.pos += 1;
        }
        false
    }
}

#[derive(Clone, Copy, Default)]
struct Component {
    id: u8,
    h: usize,
    v: usize,
    quant: usize,
    dc: usize,
    ac: usize,
}

const CONST_BITS: i64 = 13;
const PASS1_BITS: i64 = 2;
const FIX_0_211164243: i64 = 1730;
const FIX_0_509795579: i64 = 4176;
const FIX_0_601344887: i64 = 4926;
const FIX_0_765366865: i64 = 6270;
const FIX_0_899976223: i64 = 7373;
const FIX_1_061594337: i64 = 8697;
const FIX_1_451774981: i64 = 11893;
const FIX_1_847759065: i64 = 15137;
const FIX_2_172734803: i64 = 17799;
const FIX_2_562915447: i64 = 20995;

#[inline]
fn descale(x: i64, n: i64) -> i64 {
    (x + (1 << (n - 1))) >> n
}

#[inline]
fn odd_parts(z1: i64, z2: i64, z3: i64, z4: i64) -> (i64, i64) {
    let tmp0 =
        z1 * -FIX_0_211164243 + z2 * FIX_1_451774981 + z3 * -FIX_2_172734803 + z4 * FIX_1_061594337;
    let tmp2 =
        z1 * -FIX_0_509795579 + z2 * -FIX_0_601344887 + z3 * FIX_0_899976223 + z4 * FIX_2_562915447;
    (tmp0, tmp2)
}

#[inline]
fn sample(x: i64) -> u8 {
    (x + 128).clamp(0, 255) as u8
}

fn idct_4x4(block: &[i32; 64], out: &mut [u8], stride: usize) {
    let mut ws = [0i64; 32];
    for col in 0..8 {
        if col == 4 {
            continue;
        }
        let c = |row: usize| block[row * 8 + col] as i64;
        if c(1) == 0 && c(2) == 0 && c(3) == 0 && c(5) == 0 && c(6) == 0 && c(7) == 0 {
            let dc = c(0) << PASS1_BITS;
            for row in 0..4 {
                ws[row * 8 + col] = dc;
            }
            continue;
        }
        let tmp0 = c(0) << (CONST_BITS + 1);
        let tmp2 = c(2) * FIX_1_847759065 + c(6) * -FIX_0_765366865;
        let (tmp10, tmp12) = (tmp0 + tmp2, tmp0 - tmp2);
        let (odd0, odd2) = odd_parts(c(7), c(5), c(3), c(1));
        let shift = CONST_BITS - PASS1_BITS + 1;
        ws[col] = descale(tmp10 + odd2, shift);
        ws[24 + col] = descale(tmp10 - odd2, shift);
        ws[8 + col] = descale(tmp12 + odd0, shift);
        ws[16 + col] = descale(tmp12 - odd0, shift);
    }
    for row in 0..4 {
        let w = &ws[row * 8..row * 8 + 8];
        let line = &mut out[row * stride..row * stride + 4];
        if w[1] == 0 && w[2] == 0 && w[3] == 0 && w[5] == 0 && w[6] == 0 && w[7] == 0 {
            line.fill(sample(descale(w[0], PASS1_BITS + 3)));
            continue;
        }
        let tmp0 = w[0] << (CONST_BITS + 1);
        let tmp2 = w[2] * FIX_1_847759065 + w[6] * -FIX_0_765366865;
        let (tmp10, tmp12) = (tmp0 + tmp2, tmp0 - tmp2);
        let (odd0, odd2) = odd_parts(w[7], w[5], w[3], w[1]);
        let shift = CONST_BITS + PASS1_BITS + 3 + 1;
        line[0] = sample(descale(tmp10 + odd2, shift));
        line[3] = sample(descale(tmp10 - odd2, shift));
        line[1] = sample(descale(tmp12 + odd0, shift));
        line[2] = sample(descale(tmp12 - odd0, shift));
    }
}

fn be16(data: &[u8], pos: usize) -> Option<usize> {
    Some(((*data.get(pos)? as usize) << 8) | *data.get(pos + 1)? as usize)
}

pub fn decode_half(data: &[u8]) -> Option<Image> {
    decode_half_into(data, Vec::new())
}

pub fn decode_half_into(data: &[u8], mut pixels: Vec<u8>) -> Option<Image> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut quant = [[0i32; 64]; 4];
    let mut dc_tables: [Option<Huffman>; 4] = Default::default();
    let mut ac_tables: [Option<Huffman>; 4] = Default::default();
    let mut components: Vec<Component> = Vec::new();
    let (mut width, mut height) = (0usize, 0usize);
    let mut restart_interval = 0usize;
    let mut pos = 2;
    loop {
        while *data.get(pos)? != 0xFF {
            pos += 1;
        }
        while *data.get(pos)? == 0xFF {
            pos += 1;
        }
        let marker = *data.get(pos)?;
        pos += 1;
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            continue;
        }
        if marker == 0xD9 {
            return None;
        }
        let length = be16(data, pos)?;
        if length < 2 {
            return None;
        }
        let segment = data.get(pos + 2..pos + length)?;
        match marker {
            0xC0 | 0xC1 => {
                if *segment.first()? != 8 {
                    return None;
                }
                height = be16(segment, 1)?;
                width = be16(segment, 3)?;
                let count = *segment.get(5)? as usize;
                if !(count == 1 || count == 3) {
                    return None;
                }
                for k in 0..count {
                    let item = segment.get(6 + 3 * k..9 + 3 * k)?;
                    let quant_index = item[2] as usize;
                    if quant_index > 3 {
                        return None;
                    }
                    components.push(Component {
                        id: item[0],
                        h: (item[1] >> 4) as usize,
                        v: (item[1] & 15) as usize,
                        quant: quant_index,
                        ..Component::default()
                    });
                }
            }
            0xC2 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => return None,
            0xC4 => {
                let mut at = 0;
                while at < segment.len() {
                    let class = segment[at] >> 4;
                    let index = (segment[at] & 15) as usize;
                    let counts: [u8; 16] = segment.get(at + 1..at + 17)?.try_into().ok()?;
                    let total: usize = counts.iter().map(|c| *c as usize).sum();
                    let symbols = segment.get(at + 17..at + 17 + total)?;
                    if index > 3 {
                        return None;
                    }
                    let table = Huffman::new(&counts, symbols)?;
                    if class == 0 {
                        dc_tables[index] = Some(table);
                    } else {
                        ac_tables[index] = Some(table);
                    }
                    at += 17 + total;
                }
            }
            0xDB => {
                let mut at = 0;
                while at < segment.len() {
                    let precision = segment[at] >> 4;
                    let index = (segment[at] & 15) as usize;
                    if index > 3 {
                        return None;
                    }
                    for k in 0..64 {
                        let value = if precision == 0 {
                            *segment.get(at + 1 + k)? as i32
                        } else {
                            be16(segment, at + 1 + 2 * k)? as i32
                        };
                        quant[index][ZIGZAG[k] as usize] = value;
                    }
                    at += 1 + if precision == 0 { 64 } else { 128 };
                }
            }
            0xDD => restart_interval = be16(segment, 0)?,
            0xDA => {
                let count = *segment.first()? as usize;
                if count != components.len() {
                    return None;
                }
                for k in 0..count {
                    let item = segment.get(1 + 2 * k..3 + 2 * k)?;
                    let component = components.iter_mut().find(|c| c.id == item[0])?;
                    component.dc = (item[1] >> 4) as usize;
                    component.ac = (item[1] & 15) as usize;
                    if component.dc > 3 || component.ac > 3 {
                        return None;
                    }
                }
                let spectral = segment.get(1 + 2 * count..4 + 2 * count)?;
                if spectral[0] != 0 || spectral[1] != 63 || spectral[2] != 0 {
                    return None;
                }
                let header = Header {
                    width,
                    height,
                    components: &components,
                    quant: &quant,
                    dc_tables: &dc_tables,
                    ac_tables: &ac_tables,
                    restart_interval,
                };
                return decode_scan(data, pos + length, &header, std::mem::take(&mut pixels));
            }
            _ => {}
        }
        pos += length;
    }
}

struct Header<'a> {
    width: usize,
    height: usize,
    components: &'a [Component],
    quant: &'a [[i32; 64]; 4],
    dc_tables: &'a [Option<Huffman>; 4],
    ac_tables: &'a [Option<Huffman>; 4],
    restart_interval: usize,
}

fn decode_scan(data: &[u8], start: usize, header: &Header, mut pixels: Vec<u8>) -> Option<Image> {
    let Header {
        width,
        height,
        components,
        quant,
        dc_tables,
        ac_tables,
        restart_interval,
    } = *header;
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return None;
    }
    let luma = components.first()?;
    let h_max = components.iter().map(|c| c.h).max()?;
    let v_max = components.iter().map(|c| c.v).max()?;
    if components
        .iter()
        .any(|c| c.h == 0 || c.v == 0 || c.h > 4 || c.v > 4)
        || luma.h != h_max
        || luma.v != v_max
    {
        return None;
    }
    let single = components.len() == 1;
    let (mcu_w, mcu_h) = if single {
        (8, 8)
    } else {
        (8 * h_max, 8 * v_max)
    };
    let (mcus_x, mcus_y) = (width.div_ceil(mcu_w), height.div_ceil(mcu_h));
    let (luma_h, luma_v) = if single { (1, 1) } else { (luma.h, luma.v) };
    let stride = mcus_x * luma_h * 4;
    let rows = mcus_y * luma_v * 4;
    pixels.clear();
    pixels.resize(stride * rows, 0);
    let tables: Vec<(&Huffman, &Huffman)> = components
        .iter()
        .map(|c| Some((dc_tables[c.dc].as_ref()?, ac_tables[c.ac].as_ref()?)))
        .collect::<Option<_>>()?;
    let luma_quant = &quant[luma.quant];
    let mut predictors = vec![0i32; components.len()];
    let mut bits = Bits::new(data, start);
    let mut block = [0i32; 64];
    let mut until_restart = restart_interval;
    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            if restart_interval > 0 {
                if until_restart == 0 {
                    if !bits.restart() {
                        return None;
                    }
                    predictors.fill(0);
                    until_restart = restart_interval;
                }
                until_restart -= 1;
            }
            for (index, component) in components.iter().enumerate() {
                let (dc_table, ac_table) = tables[index];
                let (blocks_h, blocks_v) = if single {
                    (1, 1)
                } else {
                    (component.h, component.v)
                };
                for by in 0..blocks_v {
                    for bx in 0..blocks_h {
                        let size = bits.decode(dc_table)? as u32;
                        predictors[index] =
                            predictors[index].wrapping_add(bits.receive_extend(size));
                        if index == 0 {
                            block.fill(0);
                            block[0] = predictors[0].wrapping_mul(luma_quant[0]);
                        }
                        let mut k = 1;
                        while k < 64 {
                            let symbol = bits.decode(ac_table)?;
                            let (run, size) = ((symbol >> 4) as usize, (symbol & 15) as u32);
                            if size == 0 {
                                if run == 15 {
                                    k += 16;
                                    continue;
                                }
                                break;
                            }
                            k += run;
                            if k > 63 {
                                return None;
                            }
                            if index == 0 {
                                let natural = ZIGZAG[k] as usize;
                                block[natural] =
                                    bits.receive_extend(size).wrapping_mul(luma_quant[natural]);
                            } else {
                                bits.take(size);
                            }
                            k += 1;
                        }
                        if index == 0 {
                            let x = (mx * luma_h + bx) * 4;
                            let y = (my * luma_v + by) * 4;
                            idct_4x4(&block, &mut pixels[y * stride + x..], stride);
                        }
                    }
                }
            }
        }
    }
    let (out_w, out_h) = (width.div_ceil(2), height.div_ceil(2));
    if out_w != stride {
        for y in 1..out_h {
            pixels.copy_within(y * stride..y * stride + out_w, y * out_w);
        }
    }
    pixels.truncate(out_w * out_h);
    Some(Image::new(out_w, out_h, 1, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imaging::{self, encode_jpeg};

    fn photo(width: usize, height: usize) -> Image {
        let mut seed = 12345u32;
        let pixels = (0..width * height * 3)
            .map(|i| {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let x = (i / 3) % width;
                let y = (i / 3) / width;
                ((x * 3 + y * 2 + (seed >> 28) as usize) % 256) as u8
            })
            .collect();
        Image::new(width, height, 3, pixels)
    }

    #[test]
    fn half_scale_luma_is_close_to_a_full_decode_averaged() {
        for (w, h) in [(800, 600), (65, 49), (8, 8), (17, 3)] {
            let jpeg = encode_jpeg(&photo(w, h), 90).unwrap();
            let ours = decode_half(&jpeg).unwrap();
            let reference = imaging::halve(&imaging::decode_gray(&jpeg).unwrap());
            assert_eq!(
                (ours.width, ours.height),
                (reference.width, reference.height)
            );
            let error: f64 = ours
                .pixels
                .iter()
                .zip(&reference.pixels)
                .map(|(a, b)| (*a as f64 - *b as f64).abs())
                .sum::<f64>()
                / ours.pixels.len() as f64;
            assert!(error < 2.5, "{w}x{h}: mean error {error}");
        }
    }

    #[test]
    fn gray_and_restart_marker_jpegs_decode() {
        let gray = Image::new(40, 24, 1, (0..960).map(|i| (i % 251) as u8).collect());
        let jpeg = encode_jpeg(&gray, 92).unwrap();
        let ours = decode_half(&jpeg).unwrap();
        assert_eq!((ours.width, ours.height), (20, 12));
        let colour = photo(100, 60);
        let mut restarted = Vec::new();
        let mut encoder = jpeg_encoder::Encoder::new(&mut restarted, 90);
        encoder.set_restart_interval(2);
        encoder
            .encode(&colour.pixels, 100, 60, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        assert!(restarted.windows(2).any(|w| w == [0xFF, 0xD1]));
        let ours_restarted = decode_half(&restarted).unwrap();
        let plain = imaging::halve(&imaging::decode_gray(&restarted).unwrap());
        let gap = ours_restarted
            .pixels
            .iter()
            .zip(&plain.pixels)
            .map(|(a, b)| a.abs_diff(*b) as u32)
            .sum::<u32>() as f64
            / plain.pixels.len() as f64;
        assert!(gap < 2.5, "{gap}");
        let reference = imaging::halve(&imaging::decode_gray(&jpeg).unwrap());
        let worst = ours
            .pixels
            .iter()
            .zip(&reference.pixels)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst < 40, "{worst}");
    }

    #[test]
    fn progressive_huge_and_broken_inputs_are_refused() {
        assert!(decode_half(b"").is_none());
        assert!(decode_half(b"\xff\xd8\xff\xd9").is_none());
        let jpeg = encode_jpeg(&photo(64, 48), 90).unwrap();
        for cut in [10, 200, jpeg.len() / 2] {
            let _ = decode_half(&jpeg[..cut]);
        }
        let mut huge = jpeg.clone();
        let sof = huge.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        huge[sof + 5..sof + 9].copy_from_slice(&[0x3E, 0x80, 0x3E, 0x80]);
        assert!(decode_half(&huge).is_none());
        let mut progressive = jpeg;
        progressive[sof + 1] = 0xC2;
        assert!(decode_half(&progressive).is_none());
    }
}

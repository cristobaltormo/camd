use jpeg_encoder::{ColorType, Encoder, SamplingFactor};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn new(width: usize, height: usize, channels: usize, pixels: Vec<u8>) -> Image {
        debug_assert_eq!(pixels.len(), width * height * channels);
        Image {
            width,
            height,
            channels,
            pixels,
        }
    }

    pub fn filled(width: usize, height: usize, channels: usize, value: u8) -> Image {
        Image::new(
            width,
            height,
            channels,
            vec![value; width * height * channels],
        )
    }

    pub fn fill_rect(&mut self, x0: usize, y0: usize, x1: usize, y1: usize, value: u8) {
        let (x1, y1) = (x1.min(self.width), y1.min(self.height));
        for y in y0.min(y1)..y1 {
            let row = y * self.width * self.channels;
            self.pixels[row + x0.min(x1) * self.channels..row + x1 * self.channels].fill(value);
        }
    }
}

pub const MAX_DIMENSION: usize = 2592;

fn options(colorspace: ColorSpace) -> DecoderOptions {
    DecoderOptions::default()
        .jpeg_set_out_colorspace(colorspace)
        .set_max_width(MAX_DIMENSION)
        .set_max_height(MAX_DIMENSION)
}

fn guarded<T>(work: impl FnOnce() -> Option<T>) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .ok()
        .flatten()
}

pub fn decode_gray(jpeg: &[u8]) -> Option<Image> {
    guarded(|| {
        let mut decoder =
            JpegDecoder::new_with_options(ZCursor::new(jpeg), options(ColorSpace::Luma));
        let pixels = decoder.decode().ok()?;
        let info = decoder.info()?;
        let (width, height) = (info.width as usize, info.height as usize);
        (width > 0 && height > 0 && pixels.len() == width * height)
            .then(|| Image::new(width, height, 1, pixels))
    })
}

pub fn halve(gray: &Image) -> Image {
    let (w, h) = (gray.width, gray.height);
    let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
    let mut pixels = vec![0u8; hw * hh];
    for (y, row) in pixels.chunks_mut(hw).enumerate() {
        let top = &gray.pixels[2 * y * w..(2 * y + 1) * w];
        let bottom_index = (2 * y + 1).min(h - 1);
        let bottom = &gray.pixels[bottom_index * w..(bottom_index + 1) * w];
        for (x, out) in row.iter_mut().enumerate() {
            let (left, right) = (2 * x, (2 * x + 1).min(w - 1));
            let total =
                top[left] as u32 + top[right] as u32 + bottom[left] as u32 + bottom[right] as u32;
            *out = ((total + 2) / 4) as u8;
        }
    }
    Image::new(hw, hh, 1, pixels)
}

pub fn decode_gray_half(jpeg: &[u8]) -> Option<Image> {
    decode_gray_half_into(jpeg, Vec::new())
}

pub fn decode_gray_half_into(jpeg: &[u8], buffer: Vec<u8>) -> Option<Image> {
    guarded(|| crate::luma::decode_half_into(jpeg, buffer))
        .or_else(|| decode_gray(jpeg).map(|gray| halve(&gray)))
}

pub fn decode_rgb(jpeg: &[u8]) -> Option<Image> {
    guarded(|| {
        let mut decoder =
            JpegDecoder::new_with_options(ZCursor::new(jpeg), options(ColorSpace::RGB));
        let pixels = decoder.decode().ok()?;
        let info = decoder.info()?;
        let (width, height) = (info.width as usize, info.height as usize);
        (width > 0 && height > 0 && pixels.len() == width * height * 3)
            .then(|| Image::new(width, height, 3, pixels))
    })
}

pub fn encode_jpeg(image: &Image, quality: u8) -> Option<Vec<u8>> {
    let color = match image.channels {
        1 => ColorType::Luma,
        3 => ColorType::Rgb,
        _ => return None,
    };
    let mut out = Vec::with_capacity(image.pixels.len() / 8);
    let mut encoder = Encoder::new(&mut out, quality);
    encoder.set_sampling_factor(SamplingFactor::F_2_2);
    encoder
        .encode(
            &image.pixels,
            u16::try_from(image.width).ok()?,
            u16::try_from(image.height).ok()?,
            color,
        )
        .ok()?;
    Some(out)
}

struct Tap {
    index: usize,
    weight: f32,
}

fn area_taps(source: usize, target: usize) -> Vec<Vec<Tap>> {
    let scale = source as f64 / target as f64;
    (0..target)
        .map(|d| {
            let start = d as f64 * scale;
            let end = start + scale;
            let cell = scale.min(source as f64 - start);
            let mut first = start.ceil() as usize;
            let last = (end.floor() as usize).min(source - 1);
            first = first.min(last);
            let mut taps = Vec::new();
            if first as f64 - start > 1e-3 {
                taps.push(Tap {
                    index: first - 1,
                    weight: ((first as f64 - start) / cell) as f32,
                });
            }
            for index in first..last {
                taps.push(Tap {
                    index,
                    weight: (1.0 / cell) as f32,
                });
            }
            if end - last as f64 > 1e-3 {
                taps.push(Tap {
                    index: last,
                    weight: ((end - last as f64).min(1.0).min(cell) / cell) as f32,
                });
            }
            taps
        })
        .collect()
}

fn linear_taps(source: usize, target: usize) -> Vec<Vec<Tap>> {
    let scale = source as f64 / target as f64;
    (0..target)
        .map(|d| {
            let position = ((d as f64 + 0.5) * scale - 0.5).max(0.0);
            let left = (position.floor() as usize).min(source - 1);
            let right = (left + 1).min(source - 1);
            let fraction = (position - left as f64).clamp(0.0, 1.0) as f32;
            vec![
                Tap {
                    index: left,
                    weight: 1.0 - fraction,
                },
                Tap {
                    index: right,
                    weight: fraction,
                },
            ]
        })
        .collect()
}

pub fn resize_area(image: &Image, width: usize, height: usize) -> Image {
    let channels = image.channels;
    if width == 0 || height == 0 || image.width == 0 || image.height == 0 {
        return Image::new(width, height, channels, vec![0; width * height * channels]);
    }
    if width == image.width && height == image.height {
        return image.clone();
    }
    let shrinking = image.width >= width && image.height >= height;
    let (columns, rows) = if shrinking {
        (
            area_taps(image.width, width),
            area_taps(image.height, height),
        )
    } else {
        (
            linear_taps(image.width, width),
            linear_taps(image.height, height),
        )
    };
    let stride = width * channels;
    let mut horizontal = vec![0f32; stride * image.height];
    for y in 0..image.height {
        let source = &image.pixels[y * image.width * channels..(y + 1) * image.width * channels];
        let target = &mut horizontal[y * stride..(y + 1) * stride];
        for (x, taps) in columns.iter().enumerate() {
            for c in 0..channels {
                target[x * channels + c] = taps
                    .iter()
                    .map(|t| source[t.index * channels + c] as f32 * t.weight)
                    .sum();
            }
        }
    }
    let mut pixels = vec![0u8; stride * height];
    let mut row = vec![0f32; stride];
    for (y, taps) in rows.iter().enumerate() {
        row.fill(0.0);
        for tap in taps {
            let source = &horizontal[tap.index * stride..(tap.index + 1) * stride];
            for (acc, value) in row.iter_mut().zip(source) {
                *acc += value * tap.weight;
            }
        }
        for (out, value) in pixels[y * stride..(y + 1) * stride].iter_mut().zip(&row) {
            *out = value.round_ties_even().clamp(0.0, 255.0) as u8;
        }
    }
    Image::new(width, height, channels, pixels)
}

pub const KERNEL_FRACTION_BITS: u32 = 8;

fn round_half_even(value: f64) -> f64 {
    let rounded = value.round();
    if (value - value.trunc()).abs() == 0.5 {
        2.0 * (value / 2.0).round()
    } else {
        rounded
    }
}

pub fn gaussian_kernel(size: usize) -> Vec<u16> {
    let sigma = (size as f64).mul_add(0.15, 0.35);
    let scale = -0.125 / (sigma * sigma);
    let half = (size - 1) / 2;
    let mut values = Vec::with_capacity(half);
    let mut sum = 0.0f64;
    for i in 0..half {
        let x = 1 - size as i64 + 2 * i as i64;
        let t = ((x * x) as f64 * scale).exp();
        values.push(t);
        sum += t;
    }
    sum *= 2.0;
    sum += 1.0;
    if size.is_multiple_of(2) {
        sum += 1.0;
    }
    let normalise = 1.0 / sum;
    let one = (1u64 << KERNEL_FRACTION_BITS) as f64;
    let mut kernel = vec![0u16; size];
    let (mut error, mut total) = (0.0f64, 0i64);
    for (i, value) in values.iter().enumerate() {
        let adjusted = value * normalise * one + error;
        let rounded = round_half_even(adjusted);
        error = adjusted - rounded;
        kernel[i] = rounded as u16;
        kernel[size - 1 - i] = rounded as u16;
        total += rounded as i64;
    }
    kernel[half] = (one as i64 - 2 * total) as u16;
    kernel
}

fn reflect101(index: isize, length: usize) -> usize {
    let length = length as isize;
    if length == 1 {
        return 0;
    }
    let mut i = index;
    loop {
        if i < 0 {
            i = -i;
        } else if i >= length {
            i = 2 * length - 2 - i;
        } else {
            return i as usize;
        }
    }
}

#[derive(Default)]
pub struct BlurScratch {
    padded: Vec<u16>,
    horizontal: Vec<u16>,
    row: Vec<u32>,
}

pub fn gaussian_blur(gray: &Image, kernel: &[u16]) -> Image {
    let mut out = Vec::new();
    gaussian_blur_into(gray, kernel, &mut BlurScratch::default(), &mut out);
    Image::new(gray.width, gray.height, 1, out)
}

pub fn gaussian_blur_into(
    gray: &Image,
    kernel: &[u16],
    scratch: &mut BlurScratch,
    out: &mut Vec<u8>,
) {
    let (w, h) = (gray.width, gray.height);
    let radius = kernel.len() / 2;
    let centre = kernel[radius];
    scratch.padded.resize(w + 2 * radius, 0);
    scratch.horizontal.clear();
    scratch.horizontal.resize(w * h, 0);
    for y in 0..h {
        let source = &gray.pixels[y * w..(y + 1) * w];
        for (i, slot) in scratch.padded.iter_mut().enumerate() {
            *slot = source[reflect101(i as isize - radius as isize, w)] as u16;
        }
        let padded = &scratch.padded;
        let target = &mut scratch.horizontal[y * w..(y + 1) * w];
        for (out, value) in target.iter_mut().zip(&padded[radius..radius + w]) {
            *out = centre.wrapping_mul(*value);
        }
        for (j, weight) in kernel[..radius].iter().enumerate() {
            let left = &padded[j..j + w];
            let right = &padded[2 * radius - j..2 * radius - j + w];
            for ((out, a), b) in target.iter_mut().zip(left).zip(right) {
                *out = out.wrapping_add(weight.wrapping_mul(a.wrapping_add(*b)));
            }
        }
    }
    out.clear();
    out.resize(w * h, 0);
    scratch.row.resize(w, 0);
    let horizontal = &scratch.horizontal;
    let line = |index: isize| {
        let r = reflect101(index, h);
        &horizontal[r * w..(r + 1) * w]
    };
    for y in 0..h {
        let row = &mut scratch.row;
        for (acc, value) in row.iter_mut().zip(line(y as isize)) {
            *acc = centre as u32 * *value as u32;
        }
        for (j, weight) in kernel[..radius].iter().enumerate() {
            let offset = (radius - j) as isize;
            let (above, below) = (line(y as isize - offset), line(y as isize + offset));
            let weight = *weight as u32;
            for ((acc, a), b) in row.iter_mut().zip(above).zip(below) {
                *acc += weight * (*a as u32 + *b as u32);
            }
        }
        for (pixel, acc) in out[y * w..(y + 1) * w].iter_mut().zip(row.iter()) {
            *pixel = ((acc + (1 << 15)) >> 16).min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: usize, height: usize) -> Image {
        let pixels = (0..width * height * 3)
            .map(|i| ((i / 3) % width * 255 / width) as u8)
            .collect();
        Image::new(width, height, 3, pixels)
    }

    #[test]
    fn jpeg_roundtrip_keeps_the_size_and_the_content() {
        let image = gradient(64, 48);
        let jpeg = encode_jpeg(&image, 95).unwrap();
        let back = decode_rgb(&jpeg).unwrap();
        assert_eq!((back.width, back.height, back.channels), (64, 48, 3));
        let error: f64 = back
            .pixels
            .iter()
            .zip(&image.pixels)
            .map(|(a, b)| (*a as f64 - *b as f64).abs())
            .sum::<f64>()
            / image.pixels.len() as f64;
        assert!(error < 3.0, "mean error {error}");
    }

    #[test]
    fn half_scale_gray_decode_rounds_odd_sizes_up() {
        let jpeg = encode_jpeg(&gradient(65, 49), 90).unwrap();
        let gray = decode_gray_half(&jpeg).unwrap();
        assert_eq!((gray.width, gray.height, gray.channels), (33, 25, 1));
        assert!(gray.pixels[0] < 20 && gray.pixels[32] > 230);
    }

    #[test]
    fn images_beyond_the_dimension_cap_are_refused_before_decoding() {
        let jpeg = encode_jpeg(&gradient(64, 48), 90).unwrap();
        let mut huge = jpeg.clone();
        let sof = huge.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        huge[sof + 5..sof + 9].copy_from_slice(&[0x3E, 0x80, 0x3E, 0x80]);
        assert!(decode_rgb(&huge).is_none());
        assert!(decode_gray(&huge).is_none());
        assert!(decode_gray_half(&huge).is_none());
        assert!(decode_rgb(&jpeg).is_some());
    }

    #[test]
    fn garbage_does_not_decode() {
        assert!(decode_rgb(b"not a jpeg").is_none());
        assert!(decode_gray_half(b"").is_none());
    }

    #[test]
    fn area_resize_averages_blocks() {
        let mut image = Image::filled(4, 2, 1, 0);
        image.fill_rect(0, 0, 2, 2, 100);
        let small = resize_area(&image, 2, 1);
        assert_eq!(small.pixels, vec![100, 0]);
        let odd = resize_area(&Image::new(3, 1, 1, vec![0, 90, 180]), 2, 1);
        assert_eq!(odd.pixels, vec![30, 150]);
    }

    #[test]
    fn upscaling_interpolates() {
        let image = Image::new(2, 1, 1, vec![0, 200]);
        let big = resize_area(&image, 4, 1);
        assert_eq!(big.pixels, vec![0, 50, 150, 200]);
    }

    #[test]
    fn gaussian_kernel_is_the_opencv_fixed_point_one() {
        let kernel = gaussian_kernel(21);
        assert_eq!(kernel.iter().map(|k| *k as u32).sum::<u32>(), 256);
        assert_eq!(
            kernel,
            vec![
                0, 2, 2, 4, 6, 11, 15, 20, 25, 28, 30, 28, 25, 20, 15, 11, 6, 4, 2, 2, 0
            ]
        );
        let flat = Image::filled(30, 20, 1, 77);
        assert!(
            gaussian_blur(&flat, &kernel)
                .pixels
                .iter()
                .all(|v| *v == 77)
        );
    }

    #[test]
    fn reflect_101_mirrors_without_repeating_the_edge() {
        assert_eq!(reflect101(-1, 5), 1);
        assert_eq!(reflect101(5, 5), 3);
        assert_eq!(reflect101(-3, 1), 0);
        assert_eq!(reflect101(-12, 5), 4);
    }
}

use crate::imaging::{self, Image};
use crate::prefs::MotionSettings;

pub const WARMUP_FRAMES: u32 = 20;
pub const GLOBAL_CHANGE_FRACTION: f64 = 0.5;
pub const BACKGROUND_RATE: f32 = 0.05;
pub const BACKGROUND_RATE_DURING_MOTION: f32 = 0.01;
pub const WARMUP_RATE: f32 = 0.2;
pub const DILATE_ITERATIONS: u32 = 2;
pub const BLUR_KERNEL: usize = 21;

impl MotionSettings {
    pub fn pixel_threshold(&self) -> i64 {
        (40 - 3 * self.sensitivity).max(8)
    }

    pub fn min_area_fraction(&self) -> f64 {
        0.0015 * (11 - self.sensitivity) as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionResult {
    pub motion: bool,
    pub score: f64,
}

pub const QUIET: MotionResult = MotionResult {
    motion: false,
    score: 0.0,
};

#[derive(Default)]
struct Scratch {
    decoded: Vec<u8>,
    blurred: Vec<u8>,
    blur: imaging::BlurScratch,
    median: MedianScratch,
    changed: Vec<bool>,
}

pub struct MotionDetector {
    pub settings: MotionSettings,
    kernel: Vec<u16>,
    scratch: Scratch,
    background: Vec<f32>,
    width: usize,
    height: usize,
    seen: u32,
    mask: Option<Vec<bool>>,
    mask_key: Option<(usize, usize)>,
}

impl MotionDetector {
    pub fn new(settings: MotionSettings) -> MotionDetector {
        MotionDetector {
            settings,
            kernel: imaging::gaussian_kernel(BLUR_KERNEL),
            scratch: Scratch::default(),
            background: Vec::new(),
            width: 0,
            height: 0,
            seen: 0,
            mask: None,
            mask_key: None,
        }
    }

    pub fn update_settings(&mut self, settings: MotionSettings) {
        self.settings = settings;
        self.mask = None;
        self.mask_key = None;
    }

    pub fn reset(&mut self) {
        self.background = Vec::new();
        self.width = 0;
        self.height = 0;
        self.seen = 0;
    }

    pub fn warmed_up(&self) -> bool {
        self.seen > WARMUP_FRAMES
    }

    fn ignore_mask(&mut self, width: usize, height: usize) -> Option<&[bool]> {
        if self.mask_key != Some((width, height)) {
            self.mask_key = Some((width, height));
            self.mask = (!self.settings.ignore_rects.is_empty()).then(|| {
                let mut mask = vec![true; width * height];
                for rect in &self.settings.ignore_rects {
                    let [x, y, w, h] = match rect.as_slice() {
                        [x, y, w, h] => [*x, *y, *w, *h],
                        _ => continue,
                    };
                    let span = |start: f64, size: f64, total: usize| {
                        let total = total as f64;
                        let from = (start.max(0.0) * total) as i64;
                        let to = ((start + size).min(1.0) * total) as i64;
                        (from.max(0) as usize, to.max(0) as usize)
                    };
                    let (top, bottom) = span(y, h, height);
                    let (left, right) = span(x, w, width);
                    for row in top..bottom.min(height) {
                        let line = &mut mask[row * width..(row + 1) * width];
                        if left < right.min(width) {
                            line[left..right.min(width)].fill(false);
                        }
                    }
                }
                mask
            });
        }
        self.mask.as_deref()
    }

    fn restart(&mut self, width: usize, height: usize) -> MotionResult {
        self.background.clear();
        self.background
            .extend(self.scratch.blurred.iter().map(|v| *v as f32));
        self.width = width;
        self.height = height;
        self.seen = 1;
        QUIET
    }

    fn accumulate(&mut self, rate: f32) {
        for (bg, value) in self.background.iter_mut().zip(&self.scratch.blurred) {
            *bg = *value as f32 * rate + *bg * (1.0 - rate);
        }
    }

    pub fn process(&mut self, jpeg: &[u8]) -> MotionResult {
        let buffer = std::mem::take(&mut self.scratch.decoded);
        let Some(gray) = imaging::decode_gray_half_into(jpeg, buffer) else {
            return QUIET;
        };
        let result = self.process_gray(&gray);
        self.scratch.decoded = gray.pixels;
        result
    }

    pub fn process_gray(&mut self, gray: &Image) -> MotionResult {
        let mut blurred = std::mem::take(&mut self.scratch.blurred);
        imaging::gaussian_blur_into(gray, &self.kernel, &mut self.scratch.blur, &mut blurred);
        self.scratch.blurred = blurred;
        let (width, height) = (gray.width, gray.height);
        if self.background.is_empty() || self.width != width || self.height != height {
            return self.restart(width, height);
        }
        let gain = self
            .scratch
            .median
            .median_difference(&self.scratch.blurred, &self.background);
        let threshold = self.settings.pixel_threshold();
        let mut changed = std::mem::take(&mut self.scratch.changed);
        changed.clear();
        changed.extend(
            self.scratch
                .blurred
                .iter()
                .zip(&self.background)
                .map(|(value, bg)| {
                    let reference = (bg + gain).clamp(0.0, 255.0) as u8;
                    (value.abs_diff(reference) as i64) > threshold
                }),
        );
        if let Some(mask) = self.ignore_mask(width, height) {
            for (pixel, keep) in changed.iter_mut().zip(mask) {
                *pixel &= *keep;
            }
        }
        let count = changed.iter().filter(|c| **c).count();
        let total = changed.len();
        if count as f64 / total as f64 > GLOBAL_CHANGE_FRACTION {
            self.scratch.changed = changed;
            return self.restart(width, height);
        }
        self.seen += 1;
        if self.seen <= WARMUP_FRAMES {
            self.scratch.changed = changed;
            self.accumulate(WARMUP_RATE);
            return QUIET;
        }
        let score = if count == 0 {
            0.0
        } else {
            let grown = dilate(&changed, width, height, DILATE_ITERATIONS as usize);
            largest_contour_area(&grown, width, height) / total as f64
        };
        self.scratch.changed = changed;
        let motion = score >= self.settings.min_area_fraction();
        self.accumulate(if motion {
            BACKGROUND_RATE_DURING_MOTION
        } else {
            BACKGROUND_RATE
        });
        MotionResult { motion, score }
    }
}

const MEDIAN_BINS: usize = 512 * MEDIAN_STEPS;
const MEDIAN_STEPS: usize = 32;

fn median_bin(value: f32) -> usize {
    (((value + 256.0) * MEDIAN_STEPS as f32).max(0.0) as usize).min(MEDIAN_BINS - 1)
}

fn bin_of_rank(counts: &[u32], k: usize) -> (usize, usize) {
    let mut seen = 0usize;
    for (bin, count) in counts.iter().enumerate() {
        if seen + *count as usize > k {
            return (bin, k - seen);
        }
        seen += *count as usize;
    }
    (counts.len() - 1, 0)
}

fn select(members: &mut [f32], rank: usize) -> f32 {
    let (low, high) = members
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    if low == high {
        return low;
    }
    *members.select_nth_unstable_by(rank, f32::total_cmp).1
}

#[derive(Default)]
struct MedianScratch {
    bins: Vec<u16>,
    counts: Vec<u32>,
    members: Vec<f32>,
}

impl MedianScratch {
    fn median_difference(&mut self, gray: &[u8], background: &[f32]) -> f32 {
        let n = gray.len().min(background.len());
        if n == 0 {
            return 0.0;
        }
        self.bins.clear();
        self.bins.extend(
            gray.iter()
                .zip(background)
                .map(|(v, b)| median_bin(*v as f32 - b) as u16),
        );
        self.counts.clear();
        self.counts.resize(MEDIAN_BINS, 0);
        for bin in &self.bins {
            self.counts[*bin as usize] += 1;
        }
        let (high_bin, high_rank) = bin_of_rank(&self.counts, n / 2);
        let (low_bin, low_rank) = if n.is_multiple_of(2) {
            bin_of_rank(&self.counts, n / 2 - 1)
        } else {
            (high_bin, high_rank)
        };
        let bins = &self.bins;
        let gather = |wanted: usize, members: &mut Vec<f32>| {
            members.clear();
            for ((bin, v), b) in bins.iter().zip(gray).zip(background) {
                if *bin as usize == wanted {
                    members.push(*v as f32 - b);
                }
            }
        };
        let members = &mut self.members;
        gather(high_bin, members);
        let upper = select(members, high_rank);
        if !n.is_multiple_of(2) {
            return upper;
        }
        let lower = if low_bin == high_bin {
            select(members, low_rank)
        } else {
            gather(low_bin, members);
            select(members, low_rank)
        };
        (lower + upper) / 2.0
    }
}

pub fn median_difference(gray: &[u8], background: &[f32]) -> f32 {
    MedianScratch::default().median_difference(gray, background)
}

pub fn dilate(mask: &[bool], width: usize, height: usize, iterations: usize) -> Vec<bool> {
    let radius = iterations;
    let mut horizontal = vec![false; mask.len()];
    for y in 0..height {
        let row = &mask[y * width..(y + 1) * width];
        let out = &mut horizontal[y * width..(y + 1) * width];
        for (target, slot) in out.iter_mut().enumerate() {
            let from = target.saturating_sub(radius);
            let to = (target + radius).min(width - 1);
            *slot = row[from..=to].iter().any(|v| *v);
        }
    }
    let mut out = vec![false; mask.len()];
    for y in 0..height {
        let from = y.saturating_sub(radius);
        let to = (y + radius).min(height - 1);
        let target = &mut out[y * width..(y + 1) * width];
        for source_row in from..=to {
            let source = &horizontal[source_row * width..(source_row + 1) * width];
            for (slot, value) in target.iter_mut().zip(source) {
                *slot |= *value;
            }
        }
    }
    out
}

const NEIGHBOURS: [(isize, isize); 8] = [
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
];

fn outer_border_area(mask: &[bool], width: usize, height: usize, start: (usize, usize)) -> f64 {
    let on = |row: isize, col: isize| {
        row >= 0
            && col >= 0
            && (row as usize) < height
            && (col as usize) < width
            && mask[row as usize * width + col as usize]
    };
    let step = |p: (isize, isize), dir: usize| (p.0 + NEIGHBOURS[dir].0, p.1 + NEIGHBOURS[dir].1);
    let origin = (start.0 as isize, start.1 as isize);
    let Some(first_dir) = (0..8)
        .map(|k| (4 + 8 - k) % 8)
        .find(|dir| on(step(origin, *dir).0, step(origin, *dir).1))
    else {
        return 0.0;
    };
    let first = step(origin, first_dir);
    let mut points: Vec<(isize, isize)> = vec![origin];
    let mut previous = first;
    let mut current = origin;
    loop {
        let back = NEIGHBOURS
            .iter()
            .position(|d| (current.0 + d.0, current.1 + d.1) == previous)
            .unwrap_or(0);
        let mut next = current;
        for k in 1..=8 {
            let dir = (back + k) % 8;
            let candidate = step(current, dir);
            if on(candidate.0, candidate.1) {
                next = candidate;
                break;
            }
        }
        if next == origin && current == first {
            break;
        }
        previous = current;
        current = next;
        points.push(current);
        if points.len() > 4 * mask.len() + 8 {
            break;
        }
    }
    let mut twice = 0i64;
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        twice += (a.1 as i64) * (b.0 as i64) - (b.1 as i64) * (a.0 as i64);
    }
    twice.abs() as f64 / 2.0
}

pub fn largest_contour_area(mask: &[bool], width: usize, height: usize) -> f64 {
    let mut labelled = vec![false; mask.len()];
    let mut stack = Vec::new();
    let mut best = 0f64;
    for start in 0..mask.len() {
        if !mask[start] || labelled[start] {
            continue;
        }
        labelled[start] = true;
        stack.push(start);
        while let Some(index) = stack.pop() {
            let (row, col) = ((index / width) as isize, (index % width) as isize);
            for (dr, dc) in NEIGHBOURS {
                let (r, c) = (row + dr, col + dc);
                if r < 0 || c < 0 || r as usize >= height || c as usize >= width {
                    continue;
                }
                let neighbour = r as usize * width + c as usize;
                if mask[neighbour] && !labelled[neighbour] {
                    labelled[neighbour] = true;
                    stack.push(neighbour);
                }
            }
        }
        best = best.max(outer_border_area(
            mask,
            width,
            height,
            (start / width, start % width),
        ));
    }
    best
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::imaging::encode_jpeg;

    pub const WIDTH: usize = 800;
    pub const HEIGHT: usize = 600;

    pub fn scene_image(shift: usize, brightness: u8, blob: bool) -> Image {
        let mut img = Image::filled(WIDTH, HEIGHT, 3, brightness);
        img.fill_rect(50, 100, 251, 501, brightness.saturating_add(30));
        for k in 0..4 {
            img.fill_rect(
                400 + k * 40,
                255,
                425 + k * 40,
                305,
                brightness.saturating_sub(40),
            );
        }
        if blob {
            let x = 300 + shift;
            img.fill_rect(x, 200, x + 221, 521, 20);
        }
        img
    }

    pub fn scene(shift: usize, brightness: u8, blob: bool) -> Vec<u8> {
        encode_jpeg(&scene_image(shift, brightness, blob), 95).unwrap()
    }

    fn quiet() -> Vec<u8> {
        scene(0, 90, false)
    }

    fn warmed(detector: &mut MotionDetector, brightness: u8) {
        let frame = scene(0, brightness, false);
        for _ in 0..WARMUP_FRAMES + 5 {
            assert!(!detector.process(&frame).motion);
        }
    }

    fn settings(sensitivity: i64, ignore: Vec<Vec<f64>>) -> MotionSettings {
        MotionSettings {
            sensitivity,
            ignore_rects: ignore,
            alert_rects: Vec::new(),
        }
    }

    #[test]
    fn thresholds_follow_the_sensitivity() {
        let s = |n| settings(n, Vec::new());
        assert_eq!(s(5).pixel_threshold(), 25);
        assert_eq!(s(10).pixel_threshold(), 10);
        assert_eq!(s(1).pixel_threshold(), 37);
        assert!((s(5).min_area_fraction() - 0.009).abs() < 1e-12);
    }

    #[test]
    fn static_scene_has_no_motion() {
        let mut d = MotionDetector::new(MotionSettings::default());
        warmed(&mut d, 90);
        let frame = quiet();
        for _ in 0..30 {
            assert!(!d.process(&frame).motion);
        }
    }

    #[test]
    fn person_sized_blob_triggers() {
        let mut d = MotionDetector::new(MotionSettings::default());
        warmed(&mut d, 90);
        assert!(d.process(&scene(0, 90, true)).motion);
    }

    #[test]
    fn small_change_ignored_at_low_sensitivity() {
        let mut d = MotionDetector::new(settings(1, Vec::new()));
        warmed(&mut d, 90);
        let mut img = scene_image(0, 90, false);
        img.fill_rect(600, 300, 620, 320, 10);
        assert!(!d.process(&encode_jpeg(&img, 95).unwrap()).motion);
    }

    #[test]
    fn global_light_change_is_not_motion() {
        let mut d = MotionDetector::new(MotionSettings::default());
        warmed(&mut d, 90);
        let bright = scene(0, 200, false);
        assert!(!d.process(&bright).motion);
        for _ in 0..5 {
            assert!(!d.process(&bright).motion);
        }
    }

    #[test]
    fn ignore_rect_masks_motion() {
        let mut d = MotionDetector::new(settings(5, vec![vec![0.3, 0.3, 0.4, 0.6]]));
        warmed(&mut d, 90);
        assert!(!d.process(&scene(0, 90, true)).motion);
    }

    #[test]
    fn background_adapts_to_permanent_change() {
        let mut d = MotionDetector::new(MotionSettings::default());
        warmed(&mut d, 90);
        let moved = scene(0, 90, true);
        let results: Vec<bool> = (0..200).map(|_| d.process(&moved).motion).collect();
        assert!(results[0] && !results[199]);
    }

    #[test]
    fn slow_exposure_drift_is_not_motion() {
        let mut d = MotionDetector::new(MotionSettings::default());
        warmed(&mut d, 90);
        for step in 1..12u8 {
            assert!(!d.process(&scene(0, 90 + step * 2, false)).motion);
        }
    }

    #[test]
    fn partial_light_still_counts_as_change_when_it_is_large() {
        let mut d = MotionDetector::new(MotionSettings::default());
        warmed(&mut d, 90);
        let mut img = scene_image(0, 90, false);
        img.fill_rect(300, 150, 700, 450, 230);
        assert!(d.process(&encode_jpeg(&img, 95).unwrap()).motion);
    }

    #[test]
    fn a_change_of_resolution_restarts_the_background_instead_of_failing() {
        let mut d = MotionDetector::new(MotionSettings::default());
        warmed(&mut d, 90);
        let small = crate::imaging::resize_area(&scene_image(0, 90, false), 640, 480);
        let small = encode_jpeg(&small, 95).unwrap();
        for _ in 0..4 {
            assert!(!d.process(&small).motion);
        }
    }

    #[test]
    fn undecodable_frames_are_quiet() {
        let mut d = MotionDetector::new(MotionSettings::default());
        assert_eq!(d.process(b"garbage"), QUIET);
    }

    #[test]
    fn reset_starts_a_new_warmup() {
        let mut d = MotionDetector::new(MotionSettings::default());
        warmed(&mut d, 90);
        assert!(d.warmed_up());
        d.reset();
        assert!(!d.warmed_up());
        assert!(!d.process(&scene(0, 90, true)).motion);
    }

    #[test]
    fn median_of_an_even_count_averages_the_middle_pair() {
        assert_eq!(median_difference(&[1, 2, 3, 10], &[0.0; 4]), 2.5);
        assert_eq!(median_difference(&[5, 1, 9], &[0.0; 3]), 5.0);
        assert_eq!(
            median_difference(&[0, 0, 255, 255], &[255.0, 0.5, 0.0, 0.25]),
            127.125
        );
        let values: Vec<u8> = (0..1001u32).map(|i| (i * 7919 % 256) as u8).collect();
        let background: Vec<f32> = (0..1001u32).map(|i| (i % 13) as f32 * 0.37).collect();
        let mut sorted: Vec<f32> = values
            .iter()
            .zip(&background)
            .map(|(v, b)| *v as f32 - b)
            .collect();
        sorted.sort_by(f32::total_cmp);
        assert_eq!(median_difference(&values, &background), sorted[500]);
    }

    fn mask_from(rows: &[&str]) -> (Vec<bool>, usize, usize) {
        let width = rows[0].len();
        let mask = rows
            .iter()
            .flat_map(|r| r.chars().map(|c| c == '#'))
            .collect();
        (mask, width, rows.len())
    }

    #[test]
    fn contour_area_matches_opencv_polygons() {
        let (mask, w, h) = mask_from(&[".....", ".###.", ".###.", ".###.", "....."]);
        assert_eq!(largest_contour_area(&mask, w, h), 4.0);
        let (mask, w, h) = mask_from(&["#....", ".....", "....."]);
        assert_eq!(largest_contour_area(&mask, w, h), 0.0);
        let (mask, w, h) = mask_from(&["####", "#..#", "#..#", "####"]);
        assert_eq!(largest_contour_area(&mask, w, h), 9.0);
        let (mask, w, h) = mask_from(&["##...", "##...", "..###", "..###", "..###"]);
        assert_eq!(largest_contour_area(&mask, w, h), 5.0);
        let (mask, w, h) = mask_from(&["#.#.#", ".#.#.", "#.#.#"]);
        assert_eq!(largest_contour_area(&mask, w, h), 2.0);
        let (mask, w, h) = mask_from(&["###..", "#.#..", "#####", "..#.#", "..###"]);
        assert_eq!(largest_contour_area(&mask, w, h), 9.0);
        let (mask, w, h) = mask_from(&[".#.", "###", ".#."]);
        assert_eq!(largest_contour_area(&mask, w, h), 2.0);
    }

    #[test]
    fn dilation_grows_by_the_iterations() {
        let (mask, w, h) = mask_from(&[".......", ".......", ".......", "...#...", "......."]);
        let grown = dilate(&mask, w, h, 2);
        let on: Vec<usize> = (0..grown.len()).filter(|i| grown[*i]).collect();
        assert_eq!(on.len(), 20);
        assert!(grown[w + 1] && grown[4 * w + 5] && !grown[w] && !grown[6]);
    }
}

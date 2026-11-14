use std::path::{Path, PathBuf};

use camd::detect::{Detection, NanoDet};
use camd::imaging::{self, Image};
use camd::labels::{COCO_LABELS, DEFAULT_LABELS};
use camd::motion::MotionDetector;
use camd::prefs::MotionSettings;
use serde_json::{Value, json};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(name: &str) -> Value {
    let text = std::fs::read_to_string(root().join("tests/fixtures").join(name)).unwrap();
    serde_json::from_str(&text).unwrap()
}

struct Noise(u64);

impl Noise {
    fn next(&mut self) -> u8 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 59) as u8
    }
}

fn base_scene(brightness: u8) -> Image {
    let mut img = Image::filled(800, 600, 3, brightness);
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
    img
}

fn with_blob(mut img: Image, x: usize) -> Image {
    img.fill_rect(x, 200, x + 221, 521, 20);
    img
}

fn photo_800() -> Image {
    let scene =
        imaging::decode_rgb(&std::fs::read(root().join("docs/images/scene.jpg")).unwrap()).unwrap();
    imaging::resize_area(&scene, 800, 600)
}

fn noisy(base: &Image, noise: &mut Noise) -> Image {
    let mut img = base.clone();
    for value in img.pixels.iter_mut() {
        *value = value.saturating_add(noise.next()).saturating_sub(8);
    }
    img
}

fn jpeg(img: &Image) -> Vec<u8> {
    imaging::encode_jpeg(img, 90).unwrap()
}

struct Sequence {
    name: &'static str,
    sensitivity: i64,
    ignore: Vec<Vec<f64>>,
    frames: Vec<Vec<u8>>,
}

fn sequences() -> Vec<Sequence> {
    let quiet = jpeg(&base_scene(90));
    let warm = || vec![quiet.clone(); 25];
    let mut list = Vec::new();

    let mut frames = warm();
    frames.extend((0..10).map(|k| jpeg(&with_blob(base_scene(90), 300 + k * 4))));
    frames.extend(vec![quiet.clone(); 30]);
    list.push(Sequence {
        name: "blob",
        sensitivity: 5,
        ignore: vec![],
        frames: frames.clone(),
    });
    list.push(Sequence {
        name: "ignored",
        sensitivity: 5,
        ignore: vec![vec![0.3, 0.3, 0.4, 0.6]],
        frames,
    });

    let mut frames = warm();
    frames.extend(vec![jpeg(&base_scene(200)); 6]);
    frames.extend((1..12).map(|k| jpeg(&base_scene(200 - k * 2))));
    list.push(Sequence {
        name: "light",
        sensitivity: 5,
        ignore: vec![],
        frames,
    });

    let mut frames = warm();
    let mut small = base_scene(90);
    small.fill_rect(600, 300, 620, 320, 10);
    frames.extend(vec![jpeg(&small); 3]);
    list.push(Sequence {
        name: "small",
        sensitivity: 1,
        ignore: vec![],
        frames: frames.clone(),
    });
    list.push(Sequence {
        name: "small-sensitive",
        sensitivity: 10,
        ignore: vec![],
        frames,
    });

    let mut frames = warm();
    let mut lit = base_scene(90);
    lit.fill_rect(300, 150, 700, 450, 230);
    frames.extend(vec![jpeg(&lit); 3]);
    list.push(Sequence {
        name: "partial",
        sensitivity: 5,
        ignore: vec![],
        frames,
    });

    let mut frames = warm();
    frames.extend(vec![jpeg(&with_blob(base_scene(90), 300)); 120]);
    list.push(Sequence {
        name: "adapt",
        sensitivity: 5,
        ignore: vec![],
        frames,
    });

    let mut frames = warm();
    let reduced = imaging::resize_area(&base_scene(90), 640, 480);
    frames.extend(vec![jpeg(&reduced); 5]);
    list.push(Sequence {
        name: "resolution",
        sensitivity: 5,
        ignore: vec![],
        frames,
    });

    let photo = photo_800();
    let mut noise = Noise(7);
    let mut frames: Vec<Vec<u8>> = (0..25).map(|_| jpeg(&noisy(&photo, &mut noise))).collect();
    for k in 0..20 {
        let mut img = noisy(&photo, &mut noise);
        img.fill_rect(150 + k * 15, 180, 260 + k * 15, 520, 35);
        frames.push(jpeg(&img));
    }
    frames.extend((0..15).map(|_| jpeg(&noisy(&photo, &mut noise))));
    for sensitivity in [2, 5, 9] {
        list.push(Sequence {
            name: ["photo-2", "photo-5", "photo-9"][(sensitivity as usize) / 4],
            sensitivity,
            ignore: vec![],
            frames: frames.clone(),
        });
    }
    list
}

fn detect_inputs() -> Vec<(&'static str, Vec<u8>)> {
    let source = std::fs::read(root().join("docs/images/scene.jpg")).unwrap();
    let stream = photo_800();
    let crop = |x: f64, y: f64, w: f64, h: f64| {
        let (x0, y0) = ((x * 800.0) as usize, (y * 600.0) as usize);
        let (x1, y1) = (((x + w) * 800.0) as usize, ((y + h) * 600.0) as usize);
        let mut pixels = Vec::new();
        for row in y0..y1 {
            pixels.extend_from_slice(&stream.pixels[(row * 800 + x0) * 3..(row * 800 + x1) * 3]);
        }
        let piece = Image::new(x1 - x0, y1 - y0, 3, pixels);
        imaging::encode_jpeg(&imaging::resize_area(&piece, 640, 480), 78).unwrap()
    };
    vec![
        ("source", source),
        ("live", imaging::encode_jpeg(&stream, 80).unwrap()),
        ("right", crop(0.30, 0.0, 0.70, 0.70)),
        ("centre", crop(0.18, 0.12, 0.64, 0.64)),
        ("bed", crop(0.0, 0.18, 0.50, 0.50)),
        (
            "small",
            imaging::encode_jpeg(&imaging::resize_area(&stream, 320, 240), 80).unwrap(),
        ),
    ]
}

#[test]
#[ignore]
fn dump_parity_inputs() {
    let out = PathBuf::from(std::env::var("CAMD_PARITY_OUT").expect("set CAMD_PARITY_OUT"));
    for sequence in sequences() {
        let dir = out.join("motion").join(sequence.name);
        std::fs::create_dir_all(&dir).unwrap();
        for (i, frame) in sequence.frames.iter().enumerate() {
            std::fs::write(dir.join(format!("{i:04}.jpg")), frame).unwrap();
        }
        std::fs::write(
            dir.join("settings.json"),
            json!({"sensitivity": sequence.sensitivity, "ignore_rects": sequence.ignore})
                .to_string(),
        )
        .unwrap();
    }
    let dir = out.join("detect");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, data) in detect_inputs() {
        std::fs::write(dir.join(format!("{name}.jpg")), data).unwrap();
    }
}

#[test]
fn motion_matches_the_python_detector() {
    let expected = fixture("motion-parity.json");
    let mut report = Vec::new();
    let mut failures = Vec::new();
    for sequence in sequences() {
        let reference = expected[sequence.name].as_array().unwrap();
        assert_eq!(reference.len(), sequence.frames.len(), "{}", sequence.name);
        let mut detector = MotionDetector::new(MotionSettings {
            sensitivity: sequence.sensitivity,
            ignore_rects: sequence.ignore.clone(),
            alert_rects: Vec::new(),
        });
        let (mut worst, mut triggers, mut flips) = (0f64, 0, 0);
        let (slack, tolerance) = if sequence.sensitivity >= 9 {
            (0.004, 0.35)
        } else {
            (0.001, 0.02)
        };
        for (i, frame) in sequence.frames.iter().enumerate() {
            let result = detector.process(frame);
            let (motion, score) = (
                reference[i][0].as_bool().unwrap(),
                reference[i][1].as_f64().unwrap(),
            );
            worst = worst.max((result.score - score).abs() / score.max(0.01));
            triggers += motion as usize;
            if result.motion != motion {
                flips += 1;
                failures.push(format!(
                    "{} frame {i}: rust {:.5} {} python {score:.5} {motion}",
                    sequence.name, result.score, result.motion
                ));
            }
            if (result.score - score).abs() > slack + tolerance * score {
                failures.push(format!(
                    "{} frame {i}: score {:.5} vs python {score:.5}",
                    sequence.name, result.score
                ));
            }
        }
        report.push(format!(
            "{}: {} frames, {triggers} motion, {flips} differ, worst relative score gap {worst:.3}",
            sequence.name,
            sequence.frames.len()
        ));
    }
    eprintln!("{}", report.join("\n"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn model_path() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("CAMD_TEST_MODEL").map(PathBuf::from),
        Some(root().join("tests/models/nanodet-plus-m.onnx")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file())
}

fn compare(name: &str, ours: &[Detection], theirs: &[Value]) -> (f64, f64) {
    assert_eq!(ours.len(), theirs.len(), "{name}: {ours:?} vs {theirs:?}");
    let (mut box_gap, mut score_gap) = (0f64, 0f64);
    let mut matched = vec![false; theirs.len()];
    for d in ours {
        let (index, gap) = theirs
            .iter()
            .enumerate()
            .filter(|(i, t)| !matched[*i] && t["label"] == json!(d.label))
            .map(|(i, t)| {
                let gap = t["box"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(d.bbox)
                    .map(|(a, b)| (a.as_f64().unwrap() - b).abs())
                    .fold(0f64, f64::max);
                (i, gap)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or_else(|| panic!("{name}: no python match for {d:?}"));
        matched[index] = true;
        box_gap = box_gap.max(gap);
        score_gap =
            score_gap.max((theirs[index]["confidence"].as_f64().unwrap() - d.confidence).abs());
    }
    assert!(box_gap < 0.01, "{name}: box differs by {box_gap}");
    assert!(
        score_gap < 0.03,
        "{name}: confidence differs by {score_gap}"
    );
    (box_gap, score_gap)
}

#[test]
fn detection_matches_the_python_detector_when_the_model_is_present() {
    let Some(path) = model_path() else {
        eprintln!("skipped: no model at CAMD_TEST_MODEL or tests/models/nanodet-plus-m.onnx");
        return;
    };
    let expected = fixture("detect-parity.json");
    let model = NanoDet::load(&path).unwrap();
    let all: Vec<String> = COCO_LABELS.iter().map(|s| s.to_string()).collect();
    let defaults: Vec<String> = DEFAULT_LABELS.iter().map(|s| s.to_string()).collect();
    let (mut box_gap, mut score_gap) = (0f64, 0f64);
    for (name, data) in detect_inputs() {
        let image = imaging::decode_rgb(&data).unwrap();
        for (variant, labels, confidence) in [("all", &all, 0.3), ("default", &defaults, 0.5)] {
            let floor = if variant == "all" { 0.45 } else { 0.0 };
            let ours: Vec<Detection> = model
                .detect_image(&image, labels, confidence)
                .unwrap()
                .into_iter()
                .filter(|d| d.confidence >= floor)
                .collect();
            let theirs: Vec<Value> = expected[name][variant]
                .as_array()
                .unwrap()
                .iter()
                .filter(|d| d["confidence"].as_f64().unwrap() >= floor)
                .cloned()
                .collect();
            let theirs = &theirs;
            let (b, s) = compare(&format!("{name}/{variant}"), &ours, theirs);
            box_gap = box_gap.max(b);
            score_gap = score_gap.max(s);
        }
    }
    eprintln!("largest box difference {box_gap:.5}, largest confidence difference {score_gap:.5}");
}

#[test]
fn parity_inputs_are_deterministic() {
    let first: Vec<Vec<u8>> = detect_inputs().into_iter().map(|(_, d)| d).collect();
    let second: Vec<Vec<u8>> = detect_inputs().into_iter().map(|(_, d)| d).collect();
    assert_eq!(first, second);
    assert!(Path::new(&root().join("docs/images/scene.jpg")).is_file());
}

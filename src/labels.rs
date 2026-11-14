pub const COCO_LABELS: [&str; 80] = [
    "person",
    "bicycle",
    "car",
    "motorcycle",
    "airplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "backpack",
    "umbrella",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "couch",
    "potted plant",
    "bed",
    "dining table",
    "toilet",
    "tv",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
];

pub const DEFAULT_LABELS: [&str; 3] = ["person", "cat", "dog"];

pub fn index_of(label: &str) -> Option<usize> {
    COCO_LABELS.iter().position(|l| *l == label)
}

pub fn is_known(label: &str) -> bool {
    index_of(label).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_labels_are_known_to_the_model() {
        assert!(DEFAULT_LABELS.iter().all(|l| is_known(l)));
        assert_eq!(index_of("person"), Some(0));
        assert_eq!(index_of("toothbrush"), Some(79));
        assert_eq!(index_of("unicorn"), None);
    }
}

import json
import os
import subprocess
import sys
from pathlib import Path

import cv2

from camd.detect import ObjectDetector, ensure_model

SOURCE = Path(__file__).resolve().parent.parent / "docs" / "images" / "scene.jpg"
STREAM_SIZE = (800, 600)
CROPS = {
    "full": (0.0, 0.0, 1.0, 1.0),
    "right": (0.30, 0.0, 0.70, 0.70),
    "centre": (0.18, 0.12, 0.64, 0.64),
    "bed": (0.0, 0.18, 0.50, 0.50),
    "shelf": (0.0, 0.0, 0.45, 0.45),
}


def crop(image, spec):
    height, width = image.shape[:2]
    x, y, w, h = spec
    return image[int(y * height) : int((y + h) * height), int(x * width) : int((x + w) * width)]


def main(target: Path, model_dir: Path) -> None:
    target.mkdir(parents=True, exist_ok=True)
    image = cv2.imread(str(SOURCE))
    stream = cv2.resize(image, STREAM_SIZE, interpolation=cv2.INTER_AREA)
    cv2.imwrite(str(target / "live.jpg"), stream, [cv2.IMWRITE_JPEG_QUALITY, 80])
    for name, spec in CROPS.items():
        piece = cv2.resize(crop(stream, spec), (640, 480), interpolation=cv2.INTER_AREA)
        cv2.imwrite(str(target / f"{name}.jpg"), piece, [cv2.IMWRITE_JPEG_QUALITY, 78])
    subprocess.run(
        [
            os.environ.get("FFMPEG", "ffmpeg"),
            "-y",
            "-loglevel",
            "error",
            "-loop",
            "1",
            "-i",
            str(target / "live.jpg"),
            "-t",
            "4",
            "-r",
            "10",
            "-c:v",
            "libvpx",
            "-b:v",
            "400k",
            str(target / "clip.webm"),
        ],
        check=True,
    )
    detector = ObjectDetector(ensure_model(model_dir), min_confidence=0.4)
    found = [d.as_dict() for d in detector.detect_image(stream)]
    (target / "detections.json").write_text(json.dumps(found, indent=2))
    print(found)


if __name__ == "__main__":
    main(Path(sys.argv[1]), Path(sys.argv[2]))

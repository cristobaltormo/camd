import cv2
import numpy as np


def jpeg(image: np.ndarray) -> bytes:
    ok, buf = cv2.imencode(".jpg", image)
    assert ok
    return buf.tobytes()


def scene(shift: int = 0, brightness: int = 90, blob: bool = False) -> np.ndarray:
    img = np.full((600, 800, 3), brightness, np.uint8)
    cv2.rectangle(img, (50, 100), (250, 500), (brightness + 30,) * 3, -1)
    cv2.putText(img, "door", (400, 300), cv2.FONT_HERSHEY_SIMPLEX, 2, (brightness - 40,) * 3, 4)
    if blob:
        x = 300 + shift
        cv2.rectangle(img, (x, 200), (x + 220, 520), (20, 20, 20), -1)
    return img

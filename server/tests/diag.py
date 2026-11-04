"""Per-second brightness / motion score trace from the real camera (run on the LXC, service stopped)."""

import os
import sys
import time
from pathlib import Path

import cv2
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from camd.grabber import Grabber
from camd.motion import MotionDetector

det = MotionDetector()
bucket = {"n": 0, "bright": 0.0, "score": 0.0, "motion": 0}
last = time.time()


def on_frame(ts, jpeg):
    global last
    r = det.process(jpeg)
    g = cv2.imdecode(np.frombuffer(jpeg, np.uint8), cv2.IMREAD_REDUCED_GRAYSCALE_8)
    bucket["n"] += 1
    bucket["bright"] += float(g.mean())
    bucket["score"] = max(bucket["score"], r.score)
    bucket["motion"] += r.motion
    if ts - last >= 2:
        n = max(bucket["n"], 1)
        print(
            f"t={ts % 1000:6.0f} fps={n / 2:4.1f} bright={bucket['bright'] / n:6.1f} maxscore={bucket['score']:.3f} motionframes={bucket['motion']}",
            flush=True,
        )
        bucket.update(n=0, bright=0.0, score=0.0, motion=0)
        last = ts


g = Grabber(os.environ.get("CAM_HOST", "192.168.1.93"), os.environ["CAMD_CAMERA_TOKEN"], on_frame)
g.start()
time.sleep(int(sys.argv[1]) if len(sys.argv) > 1 else 90)
g.stop()

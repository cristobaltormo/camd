import argparse
import os
import sys
import tempfile
import time
from pathlib import Path

import cv2
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "server"))

from camd.config import Config  # noqa: E402
from camd.db import Database  # noqa: E402
from camd.grabber import CameraClient, Grabber  # noqa: E402
from camd.motion import MotionDetector  # noqa: E402
from camd.runtime import Runtime  # noqa: E402

REPORT_EVERY_S = 2


def client_from_environment() -> CameraClient:
    return CameraClient(os.environ["CAM_HOST"], os.environ["CAMD_CAMERA_TOKEN"])


def trace(seconds: int) -> None:
    detector = MotionDetector()
    bucket = {"frames": 0, "brightness": 0.0, "score": 0.0, "motion": 0, "since": time.time()}

    def on_frame(ts: float, jpeg: bytes) -> None:
        result = detector.process(jpeg)
        gray = cv2.imdecode(np.frombuffer(jpeg, np.uint8), cv2.IMREAD_REDUCED_GRAYSCALE_8)
        bucket["frames"] += 1
        bucket["brightness"] += float(gray.mean())
        bucket["score"] = max(bucket["score"], result.score)
        bucket["motion"] += result.motion
        if ts - bucket["since"] >= REPORT_EVERY_S:
            frames = max(bucket["frames"], 1)
            print(
                f"fps={frames / REPORT_EVERY_S:4.1f} brightness={bucket['brightness'] / frames:6.1f} "
                f"max_score={bucket['score']:.3f} motion_frames={bucket['motion']}",
                flush=True,
            )
            bucket.update(frames=0, brightness=0.0, score=0.0, motion=0, since=ts)

    grabber = Grabber(client_from_environment(), on_frame)
    grabber.start()
    time.sleep(seconds)
    grabber.stop()


def soak(seconds: int) -> None:
    cfg = Config(
        camera_host=os.environ["CAM_HOST"],
        camera_token=os.environ["CAMD_CAMERA_TOKEN"],
        data_dir=Path(tempfile.mkdtemp(prefix="camd-soak-")),
    )
    db = Database(cfg.db_path)
    runtime = Runtime(cfg, db)
    runtime.arming.set_mode("armed")
    runtime.analyzer.start()
    runtime.grabber.start()
    time.sleep(seconds)
    runtime.grabber.stop()
    runtime.recorder.close_open_event()
    runtime.recorder.flush()
    events = db.query("SELECT id, start_ts, end_ts, score, labels, analyzed FROM events")
    print(f"{seconds}s armed: {len(events)} events, fps {runtime.grabber.fps:.1f}, detector {runtime.analyzer.state}")
    for row in events:
        print(f"  event {row['id']}: {(row['end_ts'] or 0) - row['start_ts']:.1f}s score {row['score']:.4f} labels {row['labels']}")
    runtime.analyzer.stop()


def main() -> None:
    parser = argparse.ArgumentParser(description="Checks against a real camera. Stop the camd service first: the camera serves one stream client at a time.")
    parser.add_argument("command", choices=("trace", "soak"), help="trace: brightness and motion per second; soak: armed run that counts events in a quiet room")
    parser.add_argument("--seconds", type=int, default=90)
    args = parser.parse_args()
    {"trace": trace, "soak": soak}[args.command](args.seconds)


if __name__ == "__main__":
    main()

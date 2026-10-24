import sys
import threading
import time
from http.server import ThreadingHTTPServer
from pathlib import Path

import pytest
from frames import jpeg, scene

from camd.config import Config
from camd.db import Database
from camd.detect import Detection
from camd.runtime import Runtime

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
import fake_camera  # noqa: E402

FPS = 25
QUIET = [jpeg(scene()) for _ in range(30)]
BUSY = [jpeg(scene(shift=k * 4, blob=True)) for k in range(40)]


class AlwaysPerson:
    labels = ("person", "cat", "dog")
    min_confidence = 0.5

    def detect(self, _jpeg):
        return [Detection("person", 0.9, (0.3, 0.3, 0.3, 0.5))]


@pytest.fixture
def camera():
    frames = QUIET + BUSY + QUIET * 3
    handler = fake_camera.make_handler(frames, "t", FPS, time.time())
    servers = [ThreadingHTTPServer(("127.0.0.1", 0), handler) for _ in range(2)]
    for server in servers:
        threading.Thread(target=server.serve_forever, daemon=True).start()
    yield servers[0].server_address[1], servers[1].server_address[1]
    for server in servers:
        server.shutdown()
    fake_camera.STATE.update(standby=False)


def wait_for(condition, seconds):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if condition():
            return True
        time.sleep(0.2)
    return False


def run_pipeline(tmp_path, camera, detection, detector):
    cfg = Config(
        data_dir=tmp_path,
        camera_host="127.0.0.1",
        camera_api_port=camera[0],
        camera_stream_port=camera[1],
        camera_token="t",
        detection=detection,
        post_roll_s=3.0,
        pre_roll_s=1.0,
        mqtt_host="",
    )
    db = Database(cfg.db_path)
    runtime = Runtime(cfg, db)
    if detector is not None:
        runtime.analyzer._detector, runtime.analyzer.state = detector, "ready"
    runtime.arming.set_mode("armed")
    runtime.start()
    try:
        wait_for(lambda: db.one("SELECT 1 FROM events WHERE status='done'") is not None, 25)
    finally:
        runtime.stop()
    return db


def test_motion_from_a_real_stream_becomes_a_labelled_clip(tmp_path, camera):
    db = run_pipeline(tmp_path, camera, "alert", AlwaysPerson())
    row = db.one("SELECT * FROM events WHERE status='done'")
    assert row is not None and row["labels"] == '["person"]' and row["confirmed"] == 1
    assert Path(row["clip_path"]).stat().st_size > 1000 and Path(row["thumb_path"]).exists()


def test_without_a_detector_every_motion_is_kept(tmp_path, camera):
    db = run_pipeline(tmp_path, camera, "off", None)
    row = db.one("SELECT * FROM events WHERE status='done'")
    assert row is not None and row["analyzed"] == 0 and row["labels"] == "[]"

"""Live smoke test against the real camera with a throwaway data dir. Run on the LXC with the service stopped."""
import os
import resource
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from fastapi.testclient import TestClient

from authenticator import SoftAuthenticator
from camd.app import create_app
from camd.config import Config

ORIGIN = "http://localhost:8181"
H = {"Origin": ORIGIN}

cfg = Config(
    camera_host=os.environ.get("CAM_HOST", "192.168.1.93"),
    camera_token=os.environ["CAMD_CAMERA_TOKEN"],
    data_dir=Path(tempfile.mkdtemp(prefix="camd-smoke-")),
    rp_id="localhost",
    origin=ORIGIN,
)
app = create_app(cfg)
with TestClient(app, base_url=ORIGIN) as client:
    auth = SoftAuthenticator("localhost", ORIGIN)
    code = app.state.auth.new_enroll_code()
    opts = client.post("/api/auth/register/options", json={"code": code}, headers=H).json()
    assert client.post("/api/auth/register/verify", json={"code": code, "credential": auth.create(opts)}, headers=H).status_code == 200

    time.sleep(3)
    rt = app.state.runtime
    start_frames = rt.grabber._seq
    cpu0 = resource.getrusage(resource.RUSAGE_SELF)
    t0 = time.time()
    motion_frames = 0
    seen = start_frames
    while time.time() - t0 < 25:
        time.sleep(0.2)
        if rt.motion_now:
            motion_frames += 1
    elapsed = time.time() - t0
    cpu1 = resource.getrusage(resource.RUSAGE_SELF)
    frames = rt.grabber._seq - start_frames
    status = client.get("/api/status").json()
    snap = client.get("/api/snapshot")
    print(f"fps={frames / elapsed:.1f} frames={frames}")
    print(f"cpu={(cpu1.ru_utime + cpu1.ru_stime - cpu0.ru_utime - cpu0.ru_stime) / elapsed * 100:.0f}% of one core")
    print(f"snapshot={snap.status_code} {len(snap.content)} bytes")
    print(f"motion_polls={motion_frames} camera={status['camera']} armed={status['armed']} reason={status['reason']}")
    print(f"privacy off/on:", end=" ")
    print(client.post("/api/privacy", json={"on": True}, headers=H).status_code, end=" ")
    time.sleep(1)
    print(client.get("/api/status").json()["camera"]["standby"], end=" ")
    print(client.post("/api/privacy", json={"on": False}, headers=H).status_code)
    for _ in range(30):
        time.sleep(1)
        if client.get("/api/status").json()["camera"]["online"]:
            break
    print("back online after", _ + 1, "s:", client.get("/api/status").json()["camera"]["online"])

"""Armed soak against the real camera with a throwaway data dir: counts false events in an empty room."""

import os
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from camd.config import Config
from camd.db import Database
from camd.runtime import Runtime

seconds = int(sys.argv[1]) if len(sys.argv) > 1 else 240
cfg = Config(
    camera_host=os.environ.get("CAM_HOST", "192.168.1.93"),
    camera_token=os.environ["CAMD_CAMERA_TOKEN"],
    data_dir=Path(tempfile.mkdtemp(prefix="camd-soak-")),
)
db = Database(cfg.db_path)
rt = Runtime(cfg, db)
rt.arming.set_mode("armed")
rt.grabber.start()
polls = hits = 0
t0 = time.time()
peak = 0.0
while time.time() - t0 < seconds:
    time.sleep(0.1)
    polls += 1
    hits += rt.motion_now
rt.grabber.stop()
rt.recorder.close_open_event()
rt.recorder.flush()
events = db.query("SELECT id, start_ts, end_ts, score FROM events")
print(f"duration={seconds}s polls_with_motion={hits}/{polls} events={len(events)}")
for e in events:
    print(f"  event {e['id']} len={(e['end_ts'] or 0) - e['start_ts']:.1f}s score={e['score']:.4f}")

import time

from camd.detect import Detection
from camd.grabber import retry_delay


def test_retry_backs_off_to_thirty_seconds():
    delays = [retry_delay(n) for n in range(12)]
    assert delays[0] == delays[1] == 1.0
    assert delays == sorted(delays) and max(delays) == 30.0


def test_disabling_the_camera_is_reflected_everywhere(env):
    env.frame()
    assert env.get("/api/status").json()["camera"]["enabled"] is True
    assert env.post("/api/settings", {"camera_enabled": False}).status_code == 200
    camera = env.get("/api/status").json()["camera"]
    assert camera["enabled"] is False and camera["online"] is False
    assert not env.rt.grabber.enabled and env.rt.grabber.latest is None
    assert env.get("/api/live").status_code == 503
    assert env.get("/api/snapshot").status_code == 503
    assert env.post("/api/photos").status_code == 503
    assert env.post("/api/clips/start", {"seconds": 30}).status_code == 409
    assert env.post("/api/privacy", {"on": True}).status_code == 409
    assert env.post("/api/settings", {"camera_enabled": True}).status_code == 200
    assert env.rt.grabber.enabled


def test_frames_in_flight_are_dropped_while_disabled(env):
    grabber = env.rt.grabber
    grabber.set_enabled(False)
    env.frame()
    assert grabber.latest is None and not grabber.online
    grabber.set_enabled(True)
    env.frame()
    assert grabber.online


def test_offline_alert_is_sent_once_and_never_while_disabled(env, monkeypatch):
    sent = []
    monkeypatch.setattr(env.rt.notifier, "send", lambda title, *a, **k: sent.append(title) or 1)
    env.rt._started_at = time.time() - 3600

    env.rt.cfg.camera_enabled = False
    env.rt._check_offline()
    assert sent == []

    env.rt.apply_prefs({"camera_enabled"})
    env.rt.cfg.camera_enabled = True
    env.rt.apply_prefs({"camera_enabled"})
    env.rt._check_offline()
    assert sent == []

    env.rt._enabled_at = time.time() - 3600
    env.rt._check_offline()
    env.rt._check_offline()
    assert sent == ["Camera offline"]


def test_recovery_notice_follows_an_alert(env, monkeypatch):
    sent = []
    monkeypatch.setattr(env.rt.notifier, "send", lambda title, *a, **k: sent.append(title) or 1)
    env.rt._started_at = time.time() - 3600
    env.rt._check_offline()
    env.frame()
    env.rt._check_offline()
    env.rt._check_offline()
    assert sent == ["Camera offline", "Camera back"]


def test_a_camera_that_woke_up_is_put_back_in_standby_while_private(env, monkeypatch):
    calls = []
    monkeypatch.setattr(env.rt.grabber, "set_standby", lambda on: calls.append(on) or True)
    env.rt.grabber._standby = True
    env.rt.camera_info = {"standby": 0}
    env.rt._enforce_standby()
    assert calls == [True]
    env.rt.camera_info = {"standby": 1}
    env.rt._enforce_standby()
    assert calls == [True]


def test_notifications_are_localised_and_labelled(env, monkeypatch):
    sent = []
    monkeypatch.setattr(env.rt.notifier, "send", lambda *a, **k: sent.append(a) or 1)
    env.rt._notify = lambda title, body, event_id, tag: sent.append((title, body, event_id))
    env.cfg.language = "es"
    env.rt._event_confirmed(7, 1000.0, ["person", "cat"])
    assert sent[0][0] == "Persona, Gato detectado" and sent[0][2] == 7
    env.cfg.language = "en"
    env.rt._last_push = 0
    env.rt._event_confirmed(8, 1000.0, ["dog"])
    assert sent[1][0] == "Dog detected"


def test_unanalysed_events_notify_at_start_and_analysed_ones_wait(env):
    sent = []
    env.rt._notify = lambda title, body, event_id, tag: sent.append(title)
    env.rt._event_started(1, 1000.0, analyzed=True)
    assert sent == []
    env.rt._event_started(2, 1000.0, analyzed=False)
    assert sent == ["Motion detected"]


def test_cooldown_and_quiet_hours_silence_notifications(env):
    sent = []
    env.rt._notify = lambda title, *rest: sent.append(title)
    env.rt._event_confirmed(1, 1000.0, ["person"])
    env.rt._event_confirmed(2, 1000.0, ["person"])
    assert len(sent) == 1
    env.rt._last_push = 0
    env.cfg.notify_motion = False
    env.rt._event_confirmed(3, 1000.0, ["person"])
    assert len(sent) == 1


def test_status_exposes_fresh_detections_only(env):
    person = Detection("person", 0.8, (0.1, 0.1, 0.2, 0.4))
    env.rt.last_detections = (time.time(), [person])
    assert env.get("/api/status").json()["detection"]["items"][0]["label"] == "person"
    env.rt.last_detections = (time.time() - 60, [person])
    assert env.get("/api/status").json()["detection"]["items"] == []


def test_a_failing_result_handler_does_not_kill_the_detector_thread(tmp_path):
    from camd import detect

    class Settings:
        detection = "alert"
        detect_labels = ["person"]
        detect_confidence = 0.5
        models_dir = tmp_path

    class Fake:
        labels = ("person",)
        min_confidence = 0.5

        def detect(self, jpeg):
            return []

    calls = []

    def flaky(ts, jpeg, found):
        calls.append(ts)
        raise RuntimeError("database is locked")

    worker = detect.DetectionWorker(Settings(), flaky)
    worker._detector, worker.state = Fake(), "ready"
    worker.start()
    for ts in (1.0, 2.0):
        for _ in range(100):
            if worker.submit(ts, b"x"):
                break
            time.sleep(0.02)
        for _ in range(100):
            if ts in calls:
                break
            time.sleep(0.02)
    worker.stop()
    assert calls == [1.0, 2.0]


def test_the_last_event_card_skips_unfinished_and_unconfirmed_events(env):
    env.db.execute("INSERT INTO events(start_ts, end_ts, status, analyzed, confirmed) VALUES(100, 110, 'done', 1, 1)")
    env.db.execute("INSERT INTO events(start_ts, end_ts, status, analyzed, confirmed) VALUES(200, 210, 'done', 1, 0)")
    env.db.execute("INSERT INTO events(start_ts, status) VALUES(300, 'recording')")
    assert env.get("/api/status").json()["last_event"]["start"] == 100

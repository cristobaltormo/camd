import json
import time

from frames import jpeg, scene


def insert_event(env, start=100.0, end=110.0, labels=(), analyzed=0, clip="", thumb="", size=1):
    return env.db.execute(
        "INSERT INTO events(start_ts, end_ts, status, clip_path, thumb_path, size_bytes, labels, analyzed, confirmed) VALUES(?, ?, 'done', ?, ?, ?, ?, ?, ?)",
        (start, end, str(clip), str(thumb), size, json.dumps(list(labels)), analyzed, int(bool(labels))),
    ).lastrowid


def test_clip_range_thumb_and_delete(env):
    folder = env.cfg.clips_dir / "2026" / "10" / "12"
    folder.mkdir(parents=True)
    clip, thumb = folder / "1.mp4", folder / "1.jpg"
    clip.write_bytes(bytes(range(256)) * 40)
    thumb.write_bytes(jpeg(scene()))
    event_id = insert_event(env, clip=clip, thumb=thumb)

    event = env.get("/api/events").json()["events"][0]
    assert event["duration"] == 10.0 and event["labels"] == [] and event["detections"] == []
    full = env.get(f"/api/events/{event_id}/clip")
    assert full.status_code == 200 and full.headers["content-type"] == "video/mp4"
    part = env.get(f"/api/events/{event_id}/clip", headers={"Range": "bytes=0-99"})
    assert part.status_code == 206 and len(part.content) == 100
    assert env.get(f"/api/events/{event_id}/thumb").headers["content-type"] == "image/jpeg"

    assert env.delete(f"/api/events/{event_id}").status_code == 200
    assert not clip.exists() and env.get(f"/api/events/{event_id}/clip").status_code == 404


def test_paths_outside_the_clips_folder_are_refused(env, tmp_path_factory):
    secret = tmp_path_factory.mktemp("outside") / "secret.mp4"
    secret.write_bytes(b"secret")
    event_id = insert_event(env, clip=secret)
    assert env.get(f"/api/events/{event_id}/clip").status_code == 404


def test_keep_toggle(env):
    event_id = insert_event(env)
    assert env.post(f"/api/events/{event_id}/keep", {"keep": True}).json()["keep"] is True
    assert env.get("/api/events").json()["events"][0]["keep"] is True
    assert env.post("/api/events/999/keep", {"keep": True}).status_code == 404


def test_bulk_delete_and_unseen_count(env):
    now = time.time()
    ids = [insert_event(env, start=now - i * 100, end=now - i * 100 + 5) for i in range(3)]
    assert env.get("/api/events/count", params={"since": now - 150}).json()["count"] == 2
    assert env.post("/api/events/delete", {"ids": ids[:2] + [9999]}).json() == {"removed": 2}
    assert len(env.get("/api/events").json()["events"]) == 1


def test_label_filter_and_hidden_noise(env):
    person = insert_event(env, start=300, labels=["person"], analyzed=1)
    insert_event(env, start=200, labels=["cat"], analyzed=1)
    noise = insert_event(env, start=100, analyzed=1)
    assert [e["id"] for e in env.get("/api/events", params={"label": "person"}).json()["events"]] == [person]
    visible = env.get("/api/events", params={"hide_unconfirmed": True}).json()["events"]
    assert noise not in [e["id"] for e in visible] and len(visible) == 2
    assert env.get("/api/events/count", params={"since": 0}).json()["count"] == 2

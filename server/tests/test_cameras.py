from authenticator import SoftAuthenticator
from conftest import HEADERS, ORIGIN, enroll, make_env

from camd.config import Config, load_all


def two_cameras(tmp_path):
    second = Config(data_dir=tmp_path / "second", camera_id="porch", camera_name="Porch", rp_id="localhost", origin=ORIGIN, camera_token="u", detection="off")
    env = make_env(tmp_path / "first", extras=[second])
    enroll(env, SoftAuthenticator("localhost", ORIGIN))
    return env


def test_cameras_are_listed_and_the_cookie_selects_one(tmp_path):
    env = two_cameras(tmp_path)
    cameras = env.get("/api/cameras").json()["cameras"]
    assert [(c["id"], c["current"]) for c in cameras] == [("main", True), ("porch", False)]
    env.client.cookies.set("cam_camera", "porch")
    cameras = env.get("/api/cameras").json()["cameras"]
    assert [(c["id"], c["current"]) for c in cameras] == [("main", False), ("porch", True)]


def test_each_camera_keeps_its_own_events_and_settings(tmp_path):
    env = two_cameras(tmp_path)
    porch = env.app.state.cameras["porch"]
    porch.db.execute("INSERT INTO events(start_ts, end_ts, status, kind) VALUES(1000, 1005, 'done', 'motion')")
    assert env.get("/api/events").json()["events"] == []
    env.client.cookies.set("cam_camera", "porch")
    assert len(env.get("/api/events").json()["events"]) == 1
    assert env.client.post("/api/settings", json={"sensitivity": 9}, headers=HEADERS).status_code == 200
    env.client.cookies.set("cam_camera", "main")
    assert env.get("/api/settings").json()["sensitivity"] != 9


def test_an_unknown_camera_falls_back_to_the_first(tmp_path):
    env = two_cameras(tmp_path)
    env.client.cookies.set("cam_camera", "gone")
    assert [c["current"] for c in env.get("/api/cameras").json()["cameras"]] == [True, False]


def test_sign_in_is_shared_between_cameras(tmp_path):
    env = two_cameras(tmp_path)
    env.client.cookies.set("cam_camera", "porch")
    assert env.get("/api/status").status_code == 200


def test_extra_cameras_come_from_the_config_file(tmp_path):
    path = tmp_path / "config.toml"
    path.write_text('data_dir = "%s"\nmqtt_topic = "home"\n\n[[cameras]]\nid = "porch"\nname = "Porch"\ncamera_host = "10.0.0.9"\n' % (tmp_path / "data"))
    main, porch = load_all(str(path))
    assert main.camera_id == "main" and porch.camera_id == "porch" and porch.camera_name == "Porch"
    assert porch.camera_host == "10.0.0.9" and porch.data_dir == tmp_path / "data" / "cameras" / "porch"
    assert porch.mqtt_topic == "home_porch" and main.camera_host == ""


def test_duplicate_or_unknown_camera_entries_are_rejected(tmp_path):
    import pytest

    path = tmp_path / "config.toml"
    path.write_text('[[cameras]]\nid = "main"\n')
    with pytest.raises(ValueError):
        load_all(str(path))
    path.write_text('[[cameras]]\nid = "../x"\n')
    with pytest.raises(ValueError):
        load_all(str(path))
    path.write_text('[[cameras]]\nid = "x"\nbogus = 1\n')
    with pytest.raises(ValueError):
        load_all(str(path))

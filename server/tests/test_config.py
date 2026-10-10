from pathlib import Path

import pytest

from camd import config


def test_environment_overrides_the_file_and_is_typed(tmp_path, monkeypatch):
    path = tmp_path / "c.toml"
    path.write_text('camera_host = "10.0.0.5"\nlisten_port = 9000\n')
    monkeypatch.setenv("CAMD_CAMERA_HOST", "10.0.0.9")
    monkeypatch.setenv("CAMD_LISTEN_PORT", "9100")
    monkeypatch.setenv("CAMD_SECURE_COOKIES", "true")
    monkeypatch.setenv("CAMD_PRESENCE_DEVICES", "192.168.1.5, 192.168.1.6")
    monkeypatch.setenv("CAMD_DATA_DIR", str(tmp_path / "data"))
    cfg = config.load(str(path))
    assert cfg.camera_host == "10.0.0.9" and cfg.listen_port == 9100 and cfg.secure_cookies is True
    assert cfg.presence_devices == ["192.168.1.5", "192.168.1.6"]
    assert cfg.data_dir == Path(tmp_path / "data")


def test_a_missing_file_gives_the_defaults(tmp_path):
    cfg = config.load(str(tmp_path / "nope.toml"))
    assert cfg.listen_port == 8180 and cfg.detection == "alert" and cfg.mqtt_host == ""


def test_unknown_keys_are_refused(tmp_path):
    path = tmp_path / "c.toml"
    path.write_text("listen_prt = 1\n")
    with pytest.raises(ValueError, match="listen_prt"):
        config.load(str(path))

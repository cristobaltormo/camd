import json

import pytest

from camd import push
from camd.config import Config
from camd.db import Database
from camd.mqtt import Commands, Mqtt


class Answer:
    def __init__(self, ok=True, status_code=200):
        self.ok, self.status_code = ok, status_code


@pytest.fixture
def posts(monkeypatch):
    calls = []
    monkeypatch.setattr(push.requests, "post", lambda url, **kw: calls.append((url, kw)) or Answer())
    return calls


def notifier(tmp_path, **overrides):
    cfg = Config(data_dir=tmp_path, origin="https://cam.example.org", **overrides)
    return push.Notifier(cfg, Database(cfg.db_path))


def test_notifich_counts_as_delivered(tmp_path, posts):
    sender = notifier(tmp_path, notifich_url="https://notifich.example/", notifich_key="secret")
    assert sender.send("Title", "Body") == 1
    url, kwargs = posts[0]
    assert url == "https://notifich.example/api/notify"
    assert kwargs["headers"] == {"X-API-Key": "secret"} and kwargs["json"] == {"title": "Title", "body": "Body"}


def test_ntfy_posts_json_with_a_link_back(tmp_path, posts):
    sender = notifier(tmp_path, ntfy_url="https://ntfy.example/my-topic", ntfy_token="tk")
    assert sender.send("Title", "Body", "/#/events/3") == 1
    url, kwargs = posts[0]
    assert url == "https://ntfy.example"
    assert kwargs["json"] == {"topic": "my-topic", "title": "Title", "message": "Body", "click": "https://cam.example.org/#/events/3"}
    assert kwargs["headers"] == {"Authorization": "Bearer tk"}


def test_a_failing_channel_never_raises(tmp_path, monkeypatch):
    def boom(*_a, **_k):
        raise push.requests.ConnectionError("down")

    monkeypatch.setattr(push.requests, "post", boom)
    sender = notifier(tmp_path, notifich_url="https://x", notifich_key="k", ntfy_url="https://y/t")
    assert sender.send("a", "b") == 0


def test_unconfigured_channels_are_not_listed(tmp_path):
    assert notifier(tmp_path).channels == []


class FakeClient:
    def __init__(self):
        self.published = []

    def publish(self, topic, payload, retain=False):
        self.published.append((topic, payload))

    def subscribe(self, topic):
        self.published.append(("subscribe", topic))


def test_mqtt_discovery_announces_every_entity():
    cfg = Config(mqtt_host="broker", mqtt_topic="cam/room", site_name="Room")
    mqtt = Mqtt(cfg, Commands(lambda _: None, lambda _: None, lambda _: None))
    client = FakeClient()
    mqtt._connected(client, None, None, 0)
    configs = {topic: json.loads(payload) for topic, payload in client.published if topic.startswith("homeassistant/")}
    assert len(configs) == 6
    motion = configs["homeassistant/binary_sensor/cam_room/motion/config"]
    assert motion["state_topic"] == "cam/room/motion" and motion["device_class"] == "motion"
    assert configs["homeassistant/switch/cam_room/privacy/config"]["command_topic"] == "cam/room/privacy/set"
    assert ("cam/room/status", "online") in client.published


def test_mqtt_without_a_broker_is_inert():
    mqtt = Mqtt(Config(), Commands(lambda _: None, lambda _: None, lambda _: None))
    mqtt.publish("motion", "ON")
    mqtt.start()
    mqtt.stop()
    assert not mqtt.enabled


def test_mqtt_commands_reach_the_runtime():
    received = {}
    cfg = Config(mqtt_host="broker", mqtt_topic="cam")
    mqtt = Mqtt(cfg, Commands(lambda v: received.update(presence=v), lambda v: received.update(mode=v), lambda v: received.update(privacy=v)))

    class Message:
        def __init__(self, topic, payload):
            self.topic, self.payload = topic, payload.encode()

    mqtt._message(None, None, Message("cam/presence/set", "home"))
    mqtt._message(None, None, Message("cam/mode/set", "armed"))
    mqtt._message(None, None, Message("cam/privacy/set", "ON"))
    assert received == {"presence": True, "mode": "armed", "privacy": True}
    mqtt._message(None, None, Message("cam/presence/set", "auto"))
    assert received["presence"] is None

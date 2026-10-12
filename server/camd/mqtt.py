import logging

import paho.mqtt.client as mqtt

from .config import Config

log = logging.getLogger("camd.mqtt")


class Mqtt:
    """Publishes state to the local broker and accepts an external presence signal."""

    def __init__(self, cfg: Config, on_presence):
        self.topic = cfg.mqtt_topic
        self.on_presence = on_presence
        self.client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id="camd")
        self.client.will_set(f"{self.topic}/status", "offline", retain=True)
        self.client.on_connect = self._connected
        self.client.on_message = self._message
        self.cfg = cfg
        self._last: dict[str, str] = {}

    def start(self) -> None:
        try:
            self.client.connect_async(self.cfg.mqtt_host, self.cfg.mqtt_port, keepalive=30)
            self.client.loop_start()
        except OSError as exc:
            log.warning("mqtt unavailable: %s", exc)

    def stop(self) -> None:
        self.publish("status", "offline")
        self.client.loop_stop()
        self.client.disconnect()

    def publish(self, suffix: str, value: str) -> None:
        if self._last.get(suffix) == value:
            return
        self._last[suffix] = value
        self.client.publish(f"{self.topic}/{suffix}", value, retain=True)

    def _connected(self, client, _userdata, _flags, reason_code, _props=None):
        if reason_code != 0:
            log.warning("mqtt connect refused: %s", reason_code)
            return
        self._last.clear()
        client.subscribe(f"{self.topic}/presence/set")
        client.publish(f"{self.topic}/status", "online", retain=True)

    def _message(self, _client, _userdata, msg):
        value = msg.payload.decode(errors="ignore").strip().lower()
        if value in ("home", "away"):
            self.on_presence(value == "home")
        elif value in ("auto", "none", ""):
            self.on_presence(None)

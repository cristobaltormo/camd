import json
import logging
import re
from collections.abc import Callable
from dataclasses import dataclass

import paho.mqtt.client as mqtt

from .config import Config

log = logging.getLogger("camd.mqtt")

DISCOVERY_PREFIX = "homeassistant"
KEEPALIVE_S = 30


def binary_sensor(topic: str, state: str, device_class: str | None = None, on: str = "ON", off: str = "OFF") -> dict:
    entity = {"state_topic": f"{topic}/{state}", "payload_on": on, "payload_off": off}
    return entity | {"device_class": device_class} if device_class else entity


@dataclass
class Commands:
    presence: Callable[[bool | None], None]
    mode: Callable[[str], None]
    privacy: Callable[[bool], None]


class Mqtt:
    def __init__(self, cfg: Config, commands: Commands):
        self.cfg = cfg
        self.commands = commands
        self.topic = cfg.mqtt_topic
        self.enabled = bool(cfg.mqtt_host)
        self._last: dict[str, str] = {}
        self.client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id=f"camd-{self.topic}")
        self.client.will_set(f"{self.topic}/status", "offline", retain=True)
        self.client.on_connect = self._connected
        self.client.on_message = self._message

    def start(self) -> None:
        if not self.enabled:
            return
        try:
            self.client.connect_async(self.cfg.mqtt_host, self.cfg.mqtt_port, keepalive=KEEPALIVE_S)
            self.client.loop_start()
        except OSError as exc:
            log.warning("mqtt unavailable: %s", exc)

    def stop(self) -> None:
        if not self.enabled:
            return
        self.publish("status", "offline")
        self.client.loop_stop()
        self.client.disconnect()

    def publish(self, suffix: str, value: str) -> None:
        if not self.enabled or self._last.get(suffix) == value:
            return
        self._last[suffix] = value
        self.client.publish(f"{self.topic}/{suffix}", value, retain=True)

    def _entities(self) -> dict[str, dict]:
        t = self.topic
        return {
            "binary_sensor/motion": binary_sensor(t, "motion", "motion"),
            "binary_sensor/object": binary_sensor(t, "object", "occupancy") | {"name": "Object detected"},
            "binary_sensor/camera": binary_sensor(t, "camera", "connectivity", "online", "offline"),
            "binary_sensor/armed": binary_sensor(t, "armed"),
            "select/mode": {"state_topic": f"{t}/mode", "command_topic": f"{t}/mode/set", "options": ["auto", "armed", "disarmed"]},
            "switch/privacy": {"state_topic": f"{t}/privacy", "command_topic": f"{t}/privacy/set"},
        }

    def _announce(self, client) -> None:
        if not self.cfg.mqtt_discovery:
            return
        node = re.sub(r"\W", "_", self.topic)
        device = {"identifiers": [node], "name": self.cfg.site_name, "manufacturer": "camd"}
        for path, entity in self._entities().items():
            kind, key = path.split("/")
            payload = {
                "name": key.capitalize(),
                "unique_id": f"{node}_{key}",
                "availability_topic": f"{self.topic}/status",
                "device": device,
            } | entity
            client.publish(f"{DISCOVERY_PREFIX}/{kind}/{node}/{key}/config", json.dumps(payload), retain=True)

    def _connected(self, client, _userdata, _flags, reason_code, _props=None) -> None:
        if reason_code != 0:
            log.warning("mqtt connect refused: %s", reason_code)
            return
        self._last.clear()
        for suffix in ("presence", "mode", "privacy"):
            client.subscribe(f"{self.topic}/{suffix}/set")
        client.publish(f"{self.topic}/status", "online", retain=True)
        self._announce(client)

    def _message(self, _client, _userdata, msg) -> None:
        value = msg.payload.decode(errors="ignore").strip().lower()
        target = msg.topic.removeprefix(f"{self.topic}/").removesuffix("/set")
        try:
            if target == "presence":
                self.commands.presence(value == "home" if value in ("home", "away") else None)
            elif target == "mode":
                self.commands.mode(value)
            elif target == "privacy":
                self.commands.privacy(value in ("on", "true", "1"))
        except ValueError as exc:
            log.warning("mqtt command %s rejected: %s", target, exc)

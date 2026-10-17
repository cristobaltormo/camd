import logging
import shutil
import subprocess
import threading
import time

log = logging.getLogger("camd.presence")

PROBE_TIMEOUT_S = 5


def responds(ip: str) -> bool:
    probes = [["ping", "-c", "1", "-W", "1", ip]]
    if shutil.which("arping"):
        probes.append(["arping", "-c", "2", "-w", "2", ip])
    for probe in probes:
        try:
            if subprocess.run(probe, capture_output=True, timeout=PROBE_TIMEOUT_S).returncode == 0:
                return True
        except (OSError, subprocess.TimeoutExpired):
            continue
    return False


class Presence(threading.Thread):
    def __init__(self, devices: list[str], grace_s: float, interval: float = 30.0):
        super().__init__(daemon=True, name="presence")
        self.devices = devices
        self.grace_s = grace_s
        self.interval = interval
        self.external: bool | None = None
        self._last_seen = time.time()
        self._halt = threading.Event()

    @property
    def configured(self) -> bool:
        return bool(self.devices) or self.external is not None

    @property
    def is_home(self) -> bool:
        if self.external is not None:
            return self.external
        return time.time() - self._last_seen < self.grace_s

    def mark_seen(self) -> None:
        self._last_seen = time.time()

    def stop(self) -> None:
        self._halt.set()

    def run(self) -> None:
        while not self._halt.is_set():
            if any(responds(ip) for ip in self.devices):
                self.mark_seen()
            self._halt.wait(self.interval)

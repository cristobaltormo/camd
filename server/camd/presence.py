import logging
import shutil
import subprocess
import threading
import time

log = logging.getLogger("camd.presence")


def _responds(ip: str) -> bool:
    commands = [["ping", "-c", "1", "-W", "1", ip]]
    if shutil.which("arping"):
        commands.append(["arping", "-c", "2", "-w", "2", ip])
    for cmd in commands:
        try:
            if subprocess.run(cmd, capture_output=True, timeout=5).returncode == 0:
                return True
        except (OSError, subprocess.TimeoutExpired):
            continue
    return False


class Presence(threading.Thread):
    """Tracks whether a tracked device (his phone) is on the LAN, with a grace period before 'away'."""

    def __init__(self, devices: list[str], grace_s: float, interval: float = 30.0):
        super().__init__(daemon=True, name="presence")
        self.devices = devices
        self.grace_s = grace_s
        self.interval = interval
        self._last_seen = time.time()
        self._halt = threading.Event()
        self.external: bool | None = None

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
            for ip in self.devices:
                if _responds(ip):
                    self.mark_seen()
                    break
            self._halt.wait(self.interval)

import time

from .db import Database
from .presence import Presence

MODES = ("auto", "armed", "disarmed")


class Arming:
    """Decides whether motion should be recorded right now."""

    def __init__(self, db: Database, presence: Presence):
        self.db = db
        self.presence = presence

    @property
    def mode(self) -> str:
        return self.db.get_setting("mode", "auto")

    def set_mode(self, mode: str) -> None:
        if mode not in MODES:
            raise ValueError("invalid mode")
        self.db.set_setting("mode", mode)

    @property
    def snooze_until(self) -> float:
        return float(self.db.get_setting("snooze_until", 0))

    def snooze(self, minutes: float) -> None:
        self.db.set_setting("snooze_until", time.time() + minutes * 60 if minutes > 0 else 0)

    @property
    def privacy(self) -> bool:
        return bool(self.db.get_setting("privacy", False))

    def set_privacy(self, on: bool) -> None:
        self.db.set_setting("privacy", on)

    def decision(self) -> tuple[bool, str]:
        if self.privacy:
            return False, "privacy"
        if self.snooze_until > time.time():
            return False, "snoozed"
        mode = self.mode
        if mode == "armed":
            return True, "manual"
        if mode == "disarmed":
            return False, "manual"
        if not self.presence.configured:
            return False, "auto-without-device"
        if self.presence.is_home:
            return False, "home"
        return True, "away"

    @property
    def armed(self) -> bool:
        return self.decision()[0]

import struct
from collections.abc import Iterator
from pathlib import Path

HEADER = struct.Struct(">dI")


class Spool:
    def __init__(self, path: Path, count: int = 0, first_ts: float = 0.0, last_ts: float = 0.0):
        self.path = path
        self.count = count
        self.first_ts = first_ts
        self.last_ts = last_ts
        self._file = None

    @classmethod
    def create(cls, path: Path) -> "Spool":
        path.parent.mkdir(parents=True, exist_ok=True)
        spool = cls(path)
        spool._file = path.open("wb")
        return spool

    @classmethod
    def load(cls, path: Path) -> "Spool":
        spool = cls(path)
        for ts, _ in spool.frames():
            spool.first_ts = spool.first_ts or ts
            spool.last_ts = ts
            spool.count += 1
        return spool

    @property
    def duration(self) -> float:
        return max(self.last_ts - self.first_ts, 0.1)

    def append(self, ts: float, jpeg: bytes) -> None:
        self._file.write(HEADER.pack(ts, len(jpeg)))
        self._file.write(jpeg)
        self.first_ts = self.first_ts or ts
        self.last_ts = ts
        self.count += 1

    def close(self) -> None:
        if self._file is not None:
            self._file.close()
            self._file = None

    def frames(self) -> Iterator[tuple[float, bytes]]:
        with self.path.open("rb") as fh:
            while header := fh.read(HEADER.size):
                if len(header) < HEADER.size:
                    return
                ts, length = HEADER.unpack(header)
                jpeg = fh.read(length)
                if len(jpeg) < length:
                    return
                yield ts, jpeg

    def delete(self) -> None:
        self.close()
        self.path.unlink(missing_ok=True)

import threading


class ViewerPool:
    def __init__(self, limit: int):
        self.limit = limit
        self._open: list[threading.Event] = []
        self._lock = threading.Lock()

    def join(self) -> threading.Event:
        cancel = threading.Event()
        with self._lock:
            self._open.append(cancel)
            while len(self._open) > self.limit:
                self._open.pop(0).set()
        return cancel

    def leave(self, cancel: threading.Event) -> None:
        with self._lock:
            if cancel in self._open:
                self._open.remove(cancel)

    def __len__(self) -> int:
        with self._lock:
            return len(self._open)

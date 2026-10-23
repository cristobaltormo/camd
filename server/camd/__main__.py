import logging

import uvicorn

from .app import create_app
from .config import load

SHUTDOWN_GRACE_S = 5


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(name)s: %(message)s")
    cfg = load()
    uvicorn.run(
        create_app(cfg),
        host=cfg.listen_host,
        port=cfg.listen_port,
        access_log=False,
        log_level="warning",
        timeout_graceful_shutdown=SHUTDOWN_GRACE_S,
    )


if __name__ == "__main__":
    main()

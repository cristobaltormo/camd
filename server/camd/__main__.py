import argparse
import logging

import uvicorn

from . import __version__
from .app import create_app
from .config import load_all

SHUTDOWN_GRACE_S = 5


def main() -> None:
    parser = argparse.ArgumentParser(prog="camd", description="Self-hosted ESP32-CAM server")
    parser.add_argument("--version", action="version", version=f"camd {__version__}")
    parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(name)s: %(message)s")
    cfg, *extras = load_all()
    uvicorn.run(
        create_app(cfg, extras=extras),
        host=cfg.listen_host,
        port=cfg.listen_port,
        access_log=False,
        log_level="warning",
        timeout_graceful_shutdown=SHUTDOWN_GRACE_S,
    )


if __name__ == "__main__":
    main()

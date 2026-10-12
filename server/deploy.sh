#!/usr/bin/env bash
# Push the server tree to LXC 131 and (re)install it. Usage: ./deploy.sh [test|restart]
set -euo pipefail
cd "$(dirname "$0")"
ID=131
DEST=/opt/habitacion-cam

tar --exclude='__pycache__' --exclude='.pytest_cache' -cf - camd tests deploy.sh pyproject.toml systemd 2>/dev/null \
  | pct exec $ID -- bash -c "mkdir -p $DEST/app && tar -xf - -C $DEST/app && chown -R root:root $DEST/app"

case "${1:-}" in
  test) pct exec $ID -- bash -c "cd $DEST/app && $DEST/venv/bin/python -m pytest -q tests" ;;
  restart) pct exec $ID -- systemctl restart habitacion-cam ;;
esac

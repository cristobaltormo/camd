#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

ID="${CAMD_LXC:-131}"
DEST="${CAMD_DEST:-/opt/camd}"
SERVICE="${CAMD_SERVICE:-camd}"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

cp -r camd tests pyproject.toml stamp.py "$STAGE/"
python3 "$STAGE/stamp.py" "$STAGE/camd/web" >/dev/null

tar -C "$STAGE" --exclude='__pycache__' -cf - camd tests pyproject.toml \
  | pct exec "$ID" -- bash -c "mkdir -p $DEST/app && tar -xf - -C $DEST/app && chown -R root:root $DEST/app"

case "${1:-}" in
  test) pct exec "$ID" -- bash -c "cd $DEST/app && $DEST/venv/bin/python -m pytest -q tests" ;;
  restart) pct exec "$ID" -- systemctl restart $SERVICE ;;
esac

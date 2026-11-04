#!/usr/bin/env bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "run as root: sudo $0" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP=/opt/camd
DATA=/var/lib/camd
CONF=/etc/camd

apt-get update -qq
apt-get install -y --no-install-recommends python3 python3-venv ffmpeg iputils-ping iputils-arping

id camd >/dev/null 2>&1 || useradd --system --home "$DATA" --shell /usr/sbin/nologin camd
install -d -o camd -g camd "$DATA"
install -d "$APP" "$CONF"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp -r "$ROOT/server" "$STAGE/server"
python3 "$STAGE/server/stamp.py" "$STAGE/server/camd/web" >/dev/null

python3 -m venv "$APP/venv"
"$APP/venv/bin/pip" install --quiet --upgrade pip
"$APP/venv/bin/pip" install --quiet --upgrade "$STAGE/server" opencv-python-headless

[ -f "$CONF/config.toml" ] || install -m 0644 "$ROOT/deploy/config.example.toml" "$CONF/config.toml"
if [ ! -f "$CONF/env" ]; then
  install -m 0640 -o root -g camd /dev/null "$CONF/env"
  echo "CAMD_CAMERA_TOKEN=" > "$CONF/env"
fi
install -m 0644 "$ROOT/deploy/camd.service" /etc/systemd/system/camd.service
systemctl daemon-reload
systemctl enable camd >/dev/null

cat <<MSG

camd is installed but not started.
  1. Put the camera token in $CONF/env (CAMD_CAMERA_TOKEN=...)
  2. Edit $CONF/config.toml: camera_host, origin, rp_id
  3. Create a way to sign in:  sudo -u camd CAMD_CONFIG=$CONF/config.toml $APP/venv/bin/camd-ctl password
  4. systemctl start camd
MSG

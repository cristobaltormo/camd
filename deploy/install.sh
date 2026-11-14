#!/usr/bin/env bash
set -euo pipefail

REPO="${CAMD_REPO:-cristobaltormo/camd}"
VERSION="${CAMD_VERSION:-latest}"
BINARY="${CAMD_BINARY:-}"
BIN=/usr/local/bin/camd
CONF=/etc/camd
UNIT=/etc/systemd/system/camd.service
LEGACY_VENV=/opt/camd/venv

usage() {
  cat <<USAGE
usage: install.sh [--version vX.Y.Z] [--binary PATH]

Installs or upgrades camd as a systemd service.
  --version  release to install (default: the latest), or CAMD_VERSION
  --binary   install this camd binary instead of downloading one, or CAMD_BINARY
USAGE
}

die() {
  printf 'install.sh: %s\n' "$*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:?--version needs a value}"; shift 2 ;;
    --binary) BINARY="${2:?--binary needs a value}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

[ "$(id -u)" -eq 0 ] || die "run as root: sudo $0"
command -v systemctl >/dev/null 2>&1 || die "systemd is required; without it use the Docker image"

case "$(uname -m)" in
  x86_64|amd64) arch=amd64 ;;
  aarch64|arm64) arch=arm64 ;;
  *) [ -n "$BINARY" ] || die "there is no release binary for $(uname -m): build camd from source and pass --binary" ;;
esac
asset="camd-linux-${arch:-}"

if command -v apt-get >/dev/null 2>&1; then
  export DEBIAN_FRONTEND=noninteractive APT_LISTCHANGES_FRONTEND=none LC_ALL=C
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends ffmpeg iputils-ping iputils-arping curl ca-certificates >/dev/null
else
  for tool in ffmpeg ping curl; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is required: install it with your package manager and run this again"
  done
  command -v arping >/dev/null 2>&1 || echo "arping is not installed: presence falls back to ping only" >&2
fi

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

if [ "$VERSION" = latest ]; then
  base="https://github.com/$REPO/releases/latest/download"
else
  base="https://github.com/$REPO/releases/download/$VERSION"
fi

fetch() {
  curl -fsSL --retry 3 -o "$stage/$1" "$base/$1" || die "cannot download $base/$1"
}

verify() {
  (cd "$stage" && grep -E "^[0-9a-f]{64}  $1\$" SHA256SUMS | sha256sum -c --status -) || die "checksum of $1 does not match SHA256SUMS"
}

here=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
  here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

need_sums=0
if [ -n "$BINARY" ]; then
  [ -f "$BINARY" ] || die "$BINARY does not exist"
  cp "$BINARY" "$stage/camd"
else
  need_sums=1
fi
for file in camd.service config.example.toml; do
  if [ -n "$here" ] && [ -f "$here/$file" ]; then
    cp "$here/$file" "$stage/$file"
  else
    need_sums=1
  fi
done

if [ "$need_sums" -eq 1 ]; then
  fetch SHA256SUMS
fi
if [ -z "$BINARY" ]; then
  fetch "$asset"
  verify "$asset"
  mv "$stage/$asset" "$stage/camd"
fi
for file in camd.service config.example.toml; do
  if [ ! -f "$stage/$file" ]; then
    fetch "$file"
    verify "$file"
  fi
done

chmod 0755 "$stage/camd"
new_version="$("$stage/camd" --version 2>/dev/null)" || die "the camd binary does not run on this machine"
new_version="${new_version#camd }"

data_dir=/var/lib/camd
if [ -f "$CONF/config.toml" ]; then
  configured="$(sed -n 's/^[[:space:]]*data_dir[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' "$CONF/config.toml" | head -n 1)"
  [ -n "$configured" ] && data_dir="$configured"
fi
if [ -f "$CONF/env" ]; then
  from_env="$(sed -n 's/^CAMD_DATA_DIR=["'\'']\{0,1\}\([^"'\'']*\).*/\1/p' "$CONF/env" | tail -n 1)"
  [ -n "$from_env" ] && data_dir="$from_env"
fi

legacy=0
if { [ -f "$UNIT" ] && grep -q "$LEGACY_VENV" "$UNIT"; } || { [ ! -f "$UNIT" ] && [ -x "$LEGACY_VENV/bin/python" ]; }; then
  legacy=1
fi
previous=""
[ -x "$BIN" ] && previous="$("$BIN" --version 2>/dev/null || true)"
upgrade=0
{ [ "$legacy" -eq 1 ] || [ -n "$previous" ] || [ -f "$UNIT" ]; } && upgrade=1

was_active=0
if systemctl is-active --quiet camd 2>/dev/null; then
  was_active=1
fi

backup=""
if [ "$upgrade" -eq 1 ]; then
  backup="/var/backups/camd/$(date +%Y%m%d-%H%M%S)-before-$new_version"
  install -d -m 0700 "$backup"
  [ "$was_active" -eq 1 ] && systemctl stop camd
  [ -f "$UNIT" ] && cp -a "$UNIT" "$backup/camd.service"
  [ -d "$CONF" ] && cp -a "$CONF" "$backup/etc-camd"
  [ -x "$BIN" ] && cp -a "$BIN" "$backup/camd"
  if [ -d "$data_dir" ]; then
    (cd "$data_dir" && find . -maxdepth 3 -name 'camd.sqlite3*' -exec cp -a --parents {} "$backup/" \;) 2>/dev/null || true
    [ -f "$data_dir/vapid_private.pem" ] && cp -a "$data_dir/vapid_private.pem" "$backup/"
  fi
fi

id camd >/dev/null 2>&1 || useradd --system --home /var/lib/camd --shell /usr/sbin/nologin camd
[ -d /var/lib/camd ] || install -d -o camd -g camd -m 0750 /var/lib/camd
install -d -m 0755 "$CONF"

install -m 0755 "$stage/camd" "$BIN.new"
mv -f "$BIN.new" "$BIN"
ln -sf camd /usr/local/bin/camd-ctl

[ -f "$CONF/config.toml" ] || install -m 0640 -o root -g camd "$stage/config.example.toml" "$CONF/config.toml"
if [ ! -f "$CONF/env" ]; then
  install -m 0640 -o root -g camd /dev/null "$CONF/env"
  echo "CAMD_CAMERA_TOKEN=" > "$CONF/env"
fi
install -m 0644 "$stage/camd.service" "$UNIT"
systemctl daemon-reload
systemctl enable camd >/dev/null 2>&1

if [ "$data_dir" != /var/lib/camd ]; then
  echo "note: data_dir is $data_dir; the unit only lets camd write to /var/lib/camd. Add ReadWritePaths=$data_dir with: systemctl edit camd" >&2
fi

if [ "$upgrade" -eq 0 ]; then
  cat <<MSG

camd $new_version is installed but not started.
  1. Put the camera token in $CONF/env (CAMD_CAMERA_TOKEN=...)
  2. Edit $CONF/config.toml: camera_host and origin
  3. Create a way to sign in:  sudo -u camd CAMD_CONFIG=$CONF/config.toml camd ctl password
  4. systemctl start camd       and follow the log with: journalctl -u camd -f
MSG
  exit 0
fi

if [ "$was_active" -eq 1 ]; then
  systemctl start camd
  sleep 3
  if systemctl is-active --quiet camd; then
    echo "camd $new_version is running."
  else
    echo "camd $new_version did not stay up: journalctl -u camd -n 50" >&2
  fi
else
  echo "camd $new_version is installed. Start it with: systemctl start camd"
fi
echo "Backup of the unit, configuration and database: $backup"

if [ "$legacy" -eq 1 ]; then
  cat <<MSG

Upgraded from the Python 2.x server. The configuration, $CONF/env and the data
directory are used as they are. The old virtualenv is still in $LEGACY_VENV.
To go back to 2.x:
  systemctl stop camd
  cp $backup/camd.service $UNIT
  systemctl daemon-reload && systemctl start camd
2.x reads the database that 3.0 writes. When 3.0 has run for a while without
problems, remove the old server with: rm -rf /opt/camd
MSG
elif [ -f "$backup/camd" ]; then
  cat <<MSG

To go back to ${previous:-the previous version}:
  systemctl stop camd
  cp $backup/camd $BIN
  systemctl start camd
MSG
fi

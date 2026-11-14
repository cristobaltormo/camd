#!/usr/bin/env bash
set -euo pipefail

usage="usage: tools/ui-test.sh [test|screenshots|serve]
  test         run ui-smoke and ui-e2e against the real binary and a fake camera (default)
  screenshots  capture docs/images (OUT to change the folder, ROUTES for extra review shots)
  serve        start everything, seed it and wait (Ctrl-C stops)
Uses target/release/camd or target/debug/camd, whichever is newer (CAMD_BIN to choose), and never builds."

mode="${1:-test}"
case "$mode" in test|screenshots|serve) ;; *) echo "$usage" >&2; exit 2 ;; esac

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

bin="${CAMD_BIN:-}"
if [ -z "$bin" ]; then
  for candidate in target/release/camd target/debug/camd; do
    if [ -x "$candidate" ] && { [ -z "$bin" ] || [ "$candidate" -nt "$bin" ]; }; then bin="$candidate"; fi
  done
fi
[ -n "$bin" ] && [ -x "$bin" ] || { echo "no camd binary in target/: run cargo build first" >&2; exit 1; }
stale="$(find web -type f -newer "$bin" | head -n 1)"
[ -z "$stale" ] || { echo "$stale is newer than $bin and the interface is embedded at build time: run cargo build first" >&2; exit 1; }

if [ -z "${CHROMIUM:-}" ] && ! command -v chromium >/dev/null; then
  for candidate in chromium-browser google-chrome "$HOME"/.cache/puppeteer/chrome/*/chrome-linux64/chrome; do
    if command -v "$candidate" >/dev/null 2>&1; then export CHROMIUM="$candidate"; break; fi
  done
fi

port="${UI_PORT:-18499}"
cam_api="${UI_CAMERA_API_PORT:-18481}"
cam_stream="${UI_CAMERA_STREAM_PORT:-18482}"
base="http://127.0.0.1:$port"
password="ui-test-$RANDOM-$RANDOM"
model="${CAMD_MODEL:-$HOME/.cache/camd-model/nanodet-plus-m.onnx}"

mkdir -p target
work="$(mktemp -d "$root/target/ui-test.XXXXXX")"
pids=()
cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  if [ "${KEEP:-}" = 1 ]; then echo "kept $work" >&2; else rm -rf "$work"; fi
}
trap cleanup EXIT INT TERM

frames="$work/frames"
mkdir -p "$frames" "$work/data/models"
frame="crop=iw*0.95:ih*0.95:0:ih*0.05"
ffmpeg -loglevel error -i docs/images/scene.jpg -vf "$frame,scale=800:600" -q:v 4 "$work/still.jpg"
ffmpeg -loglevel error -loop 1 -i docs/images/scene.jpg -vf "$frame,scale=880:660,crop=800:600:40+36*sin(n/2.5):30+10*cos(n/3)" -frames:v 24 -q:v 4 "$work/move-%03d.jpg"
for i in $(seq 0 149); do
  name="$(printf '%03d' "$i")"
  if [ "$i" -ge 30 ] && [ "$i" -lt 54 ]; then mv "$work/move-$(printf '%03d' $((i - 29))).jpg" "$frames/$name.jpg"
  else ln "$work/still.jpg" "$frames/$name.jpg"; fi
done
[ -f "$model" ] && cp "$model" "$work/data/models/nanodet-plus-m.onnx"

cat > "$work/camd.toml" <<EOF
camera_host = "127.0.0.1"
camera_api_port = $cam_api
camera_stream_port = $cam_stream
camera_token = "test"
data_dir = "$work/data"
listen_host = "127.0.0.1"
listen_port = $port
origin = "$base"
rp_id = "127.0.0.1"
site_name = "Bedroom"
language = "en"
secure_cookies = false
max_event_s = 20
EOF
export CAMD_CONFIG="$work/camd.toml"

printf '%s\n%s\n' "$password" "$password" | "$bin" ctl password >/dev/null 2>&1
python3 tools/fake_camera.py "$frames" --token test --api-port "$cam_api" --stream-port "$cam_stream" >"$work/camera.log" 2>&1 &
pids+=($!)
"$bin" >"$work/camd.log" 2>&1 &
pids+=($!)
for _ in $(seq 1 100); do curl -fs "$base/healthz" >/dev/null 2>&1 && break; sleep 0.1; done
curl -fs "$base/healthz" >/dev/null || { cat "$work/camd.log" >&2; exit 1; }

case "$mode" in
  test) seed_mode=disarmed ;;
  *) seed_mode=armed ;;
esac
cookie="$(BASE="$base" PASSWORD="$password" FINAL_MODE="$seed_mode" node tools/ui-seed.mjs)" || { tail -n 30 "$work/camd.log" >&2; exit 1; }

status=0
case "$mode" in
  test)
    BASE="$base" COOKIE="$cookie" node --experimental-websocket tools/ui-smoke.mjs || status=1
    UI_BASE="$base" COOKIE="$cookie" node --experimental-websocket tools/ui-e2e.mjs || status=1
    ;;
  screenshots)
    BASE="$base" COOKIE="$cookie" node --experimental-websocket tools/screenshots.mjs || status=1
    ;;
  serve)
    echo "camd on $base, password $password, data in $work (Ctrl-C stops)"
    wait
    ;;
esac
[ "$status" = 0 ] || tail -n 30 "$work/camd.log" >&2
exit "$status"

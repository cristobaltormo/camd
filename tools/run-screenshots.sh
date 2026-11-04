#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
scenes="$(mktemp -d)"
mock=""
trap '[ -n "$mock" ] && kill "$mock" 2>/dev/null || true; rm -rf "$scenes"' EXIT
(cd "$root/server" && python "$root/tools/make_scenes.py" "$scenes" "${CAMD_MODEL_DIR:-$HOME/.cache/camd-model}" >/dev/null)
UI_MOCK_DETECT=1 UI_MOCK_SCENES="$scenes" python "$root/tools/ui-mock.py" &
mock=$!
sleep 1
cd "$root"
node --experimental-websocket tools/screenshots.mjs

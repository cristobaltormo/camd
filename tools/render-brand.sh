#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
chrome="${CHROMIUM:-chromium}"
web="$root/server/camd/web"
render() {
  local source="$1" size="$2" output="$3"
  local page
  page="$(mktemp --suffix=.html)"
  printf '<!doctype html><style>html,body{margin:0;background:transparent}img{display:block;width:%spx;height:%spx}</style><img src="file://%s">' "$size" "$size" "$source" > "$page"
  "$chrome" --headless=new --no-sandbox --disable-gpu --hide-scrollbars --default-background-color=00000000 --window-size="$size,$size" --screenshot="$output" "file://$page" >/dev/null 2>&1
  rm -f "$page"
}
render "$web/icon.svg" 512 "$web/icon-512.png"
render "$web/icon.svg" 192 "$web/icon-192.png"
render "$web/icon.svg" 180 "$web/apple-touch-icon.png"
render "$root/tools/brand/maskable.svg" 512 "$web/icon-maskable-512.png"
render "$root/tools/brand/maskable.svg" 192 "$web/icon-maskable-192.png"
"$chrome" --headless=new --no-sandbox --disable-gpu --hide-scrollbars --allow-file-access-from-files --window-size=1280,640 --screenshot="$root/docs/images/social-preview.png" "file://$root/tools/brand/social.html" >/dev/null 2>&1

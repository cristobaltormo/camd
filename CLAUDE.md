# camd

Self-hosted ESP32-CAM server: live view, motion and object detection, clips, alerts. Read `README.md` for the architecture and `docs/development.md` for the layout.

## Rules for this repo

- Code and identifiers in English. No comments in code or config: names carry the intent, reasoning goes in commit messages and docs.
- No emojis anywhere. Icons are inline SVG.
- Interface text goes through `t("English sentence")`; add the Spanish in `server/camd/web/js/es.js`. Server-side alert text lives in `camd/messages.py`.
- Every behaviour change has a test. Keep the server light: no work per frame that can be done per event, no frames kept in memory for the length of an event.
- No secrets in files, commits or messages.

## Commands

| Task | Command |
|---|---|
| Server tests | `cd server && pytest -q` (needs ffmpeg) |
| Lint and format | `cd server && ruff check . && ruff format .` |
| Interface tests | `cd server && node --test webtests/` |
| Browser tests | `python tools/ui-mock.py &` then `node --experimental-websocket tools/ui-e2e.mjs` |
| Fake camera | `python tools/fake_camera.py FRAMES_DIR --token test` |
| Firmware build | `cd firmware && pio run` |
| Run the server | `cd server && CAMD_CONFIG=... python -m camd` |

## Frontend

`server/camd/web/` has no build step: ES modules in `js/` (`main.js` shell and hash router, one module per section), `theme.js` applies the saved theme before first paint, `sw.js` caches the shell network-first. The CSP forbids inline styles and scripts: set styles with `el.style.x = ...`, never `setAttribute("style")` or `<style>`. Sources carry no cache stamps; `server/stamp.py` adds them when packaging.

## Traps

- The camera serves one stream client at a time. Never open a second consumer while the service runs: stop it before `tools/camera_check.py`.
- Opening the board's serial port resets it; wait about 15 s before asking for `status`.
- `threading.Thread` owns `_stop`: name stop flags differently.
- Passkeys need a secure origin and a matching `rp_id`.
- OpenCV's DNN module returns the network outputs in different orders between versions: `detect.pair_levels` identifies them by shape, keep it that way.
- The detection model is downloaded on first use; tests never need the network (the detector is faked).
- The ESP32 only joins 2.4 GHz.

# Development

```
firmware/        ESP32-CAM firmware (PlatformIO, Arduino)
server/camd/     the service
  api/           HTTP routers: auth, control, events, live, photos, notifications
  web/           the app: ES modules without a build step
server/tests/    pytest suite
server/webtests/ Node tests for the interface helpers and the translation catalog
tools/           fake camera, browser tests, real-camera checks
deploy/          systemd unit, proxy examples, example configs, installer (the Dockerfile is at the root)
```

## Running the tests

```bash
cd server
pip install -e '.[dev]'
pytest -q                               # FFmpeg must be installed
node --test webtests/
ruff check . && ruff format --check .
```

The pytest suite includes a pipeline test that starts a fake camera, streams real JPEG frames through the grabber, motion detector and recorder, and checks the clip FFmpeg produces.

## A camera without hardware

```bash
mkdir frames && python -c "
import cv2, numpy as np
for i in range(100):
    img = np.full((600, 800, 3), 110, np.uint8)
    cv2.rectangle(img, (100 + i * 5, 200), (260 + i * 5, 500), (30, 30, 30), -1)
    cv2.imwrite(f'frames/{i:03d}.jpg', img)
"
python tools/fake_camera.py frames --token test --fps 10 --api-port 8081 --stream-port 8082
```

Point a camd at it with `camera_host = "127.0.0.1"`, `camera_api_port = 8081`, `camera_stream_port = 8082`.

## The interface without a server

```bash
python tools/ui-mock.py                  # http://127.0.0.1:8199, fake API with state
node --experimental-websocket tools/ui-e2e.mjs
```

`ui-e2e.mjs` drives headless Chromium (set `CHROMIUM` to the binary if it is not called `chromium`). Set `UI_MOCK_IMAGE` to a JPEG and `UI_MOCK_DETECT=1` to take screenshots with a detection box.

The interface has no build step. `server/stamp.py` adds a content hash to every asset URL when the Docker image or the installer packages it, so a browser can never mix old and new files; the sources in git carry no stamps.

## Translations

User-facing text uses English sentences as keys: `t("Delete event")`. Spanish lives in `web/js/es.js`. `node --test` fails when a string has no translation, when a translation is no longer used, or when an object name of the detector has none. To add a language, create a catalog like `es.js`, register it in `text.js` and add its name to `LANGUAGES`; server-side alert texts are in `camd/messages.py`.

## Checks against a real camera

Stop the service first: the camera serves one stream client at a time.

```bash
export CAM_HOST=192.168.1.50 CAMD_CAMERA_TOKEN=...
python tools/camera_check.py trace --seconds 60    # brightness and motion score per second
python tools/camera_check.py soak --seconds 300    # armed run, counts false events
```

## Conventions

- Code and identifiers in English. No comments: names carry the intent and the reasoning belongs in the commit message or the docs.
- No emojis anywhere; icons are inline SVG.
- Interface text is never hard-coded in one language.
- Every behaviour change comes with a test.

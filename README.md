# habitacion-cam

Home camera for one room: an ESP32-CAM streams video, a small service on a Proxmox LXC
detects motion, records short clips and sends push notifications to a PWA protected by passkeys.

```
ESP32-CAM (192.168.1.93) --MJPEG, bearer token--> LXC 131 camd (127.0.0.1:8180)
                                                    |-- motion detection (OpenCV)
                                                    |-- events: pre-roll + clip (ffmpeg) + thumbnail, SQLite
                                                    |-- MQTT state (Mosquitto, local)
                                                    |-- Web Push (VAPID)
                                                    `-- PWA + API (passkey login, revocable sessions)
nginx :443 (Let's Encrypt cert for cam.cristobal.cc) --> camd
```

Only the service talks to the camera, so the camera stream is a single connection that the service fans
out to every viewer. The camera is never exposed outside the LAN.

## Layout

| Path | What |
|---|---|
| `firmware/` | PlatformIO project for the ESP32-CAM (stream, snapshot, status, control, standby, OTA) |
| `server/camd/` | The service: grabber, motion, events, arming, presence, auth, push, MQTT, API, `web/` PWA |
| `server/tests/`, `server/webtests/` | pytest suite (plus `smoke.py`, `soak.py`, `diag.py` for the real camera) and Node tests for the UI helpers |
| `server/stamp.py` | Stamps every web asset with a content hash on deploy so clients never mix versions |
| `server/systemd/`, `server/nginx/` | Unit and reverse-proxy config installed on the LXC |
| `tools/` | Serial helper, certificate renewal, `ui-mock.py` + `ui-e2e.mjs` (browser tests against a fake API), `ui-smoke.mjs` (real instance) |

## Arming

`mode` is `auto`, `armed` or `disarmed`. In `auto` the service records only when no tracked device
(his phone, `presence_devices`) answers on the LAN for `presence_grace_s`. A snooze and a privacy switch
override everything; privacy puts the camera sensor in standby (the firmware deinitialises it), so nothing
is captured at all.

## Camera unplugged

`camera_enabled` (Settings, Camera) pauses the service's search for the camera: no connection attempts, no
offline alert, no motion pipeline. With it on but the camera away, the service retries with growing delays up
to one attempt every 30 s, warns once after 5 minutes and says so when the camera is back. On reconnection the
image settings are re-applied and a privacy setting made earlier is enforced again.

## Development

```bash
# firmware (host)
cd firmware && ../.venv/bin/pio run                        # build
CAM_TOKEN=... ../.venv/bin/pio run -e esp32cam-ota -t upload   # over the air

# server
cd server && ./deploy.sh test       # copy to LXC 131 and run pytest there
./deploy.sh restart                 # restart the service after a deploy
pct exec 131 -- sudo -u camd /opt/habitacion-cam/venv/bin/python -m camd.ctl enroll   # one-time passkey code
```

First flash over USB goes through the Mac (the board is plugged there): copy the build artifacts and use
`esptool`, then provision wifi and token with `tools/serial_cmd.py` (`ssid`, `pass`, `token`, `reboot`).

## Secrets

Nothing sensitive lives in this repository. The camera token and the wifi password are in Vaultwarden;
the token reaches the service through `/etc/habitacion-cam/env` (root:camd, 0640).

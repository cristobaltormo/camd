# The camera firmware

Written for the AI-Thinker ESP32-CAM (ESP32, OV2640, 4 MB flash, PSRAM) with the Arduino framework on PlatformIO. It does a few things and nothing else: it serves the image, it can be configured, and it can be updated.

## Flashing

**Web installer.** If your fork publishes the release workflow, open its GitHub Pages site in Chrome or Edge, plug the board in and press install.

**PlatformIO.** From `firmware/`:

```bash
pip install platformio
pio run -t upload                 # USB, first time
```

The ESP32-CAM-MB base board enters flashing mode by itself. With a bare board, bridge IO0 to GND, press reset, flash, then remove the bridge.

## First start: the setup page

With no wifi configured the camera opens an access point called `camd-setup-XXXX` and a captive page. Pick your 2.4 GHz network, type its password and copy the access token (24 hex characters, generated in your browser). Save: the camera restarts and joins your wifi.

The same page comes back by itself if the camera cannot reach its wifi for two minutes, so a changed router password is recoverable without cables. Because the network is open, an already configured camera only accepts new settings when you also type its current token. If it stays offline for ten minutes it reboots.

## Finding it

The camera announces itself over mDNS as `camd-<last 3 bytes of its MAC>.local`. Most routers also show it in their client list; reserving its address there is the most reliable option, and the only one that works from inside Docker.

## Over-the-air updates

```bash
export CAM_HOST=192.168.1.50 CAM_TOKEN=<the token>
pio run -e esp32cam-ota -t upload
```

## Serial console (115200 baud)

| Command | Effect |
|---|---|
| `status` | firmware, host name, wifi state, address, reset reason |
| `scan` | list wifi networks |
| `ssid X`, `pass Y`, `token Z`, `hostname N` | store settings (the wifi ones apply after a reboot) |
| `portal` | open the setup page now |
| `reboot`, `wipe` | restart; erase every stored setting |

`tools/serial_console.py PORT status` sends commands from a script.

## HTTP API

All endpoints need `Authorization: Bearer <token>` except `/health`.

| Endpoint | Port | What it does |
|---|---|---|
| `GET /health` | 80 | `ok` |
| `GET /status` | 80 | JSON: firmware, uptime, signal, memory, current resolution, standby |
| `GET /capture` | 80 | one JPEG |
| `GET /control?var=NAME&val=N` | 80 | changes a sensor setting |
| `GET /stream` | 81 | MJPEG, one client at a time |

Controls: `framesize` (5 = QVGA up to 13 = UXGA; 8 = VGA, 9 = SVGA, 10 = XGA), `quality` (4-63, lower is better), `brightness`, `contrast`, `saturation`, `sharpness`, `ae_level` (-2..2), `vflip`, `hmirror`, `awb`, `wb_mode`, `aec`, `aec2`, `agc`, `gainceiling` (0-6), `lenc`, `led` (the flash LED on GPIO 4) and `standby` (1 shuts the sensor down).

## Robustness

A hardware watchdog restarts the board if the main loop stalls, and the camera restarts itself if it stops delivering frames. The stream and capture handlers never hold a frame longer than they need, so the next client does not wait.

## Notes

- Only the AI-Thinker pin layout is built in (`firmware/src/pins.h`). Other ESP32-CAM boards need their own pins.
- Tokens are 12 to 64 letters, digits, `-` or `_`.
- The board only joins 2.4 GHz networks. The external antenna connector matters: without the antenna fitted the signal can be 30 dB weaker.
- The OV2640 has no infrared filter removal: in the dark it sees nothing. Light the room, or use the `led` control as a night light.
- The access token is stored in the board's flash in clear text, as is the wifi password. Anyone with physical access can read them.

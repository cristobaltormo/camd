# camd

Firmware and server for an ESP32-CAM that watches one room.

## Firmware

Written for the AI-Thinker ESP32-CAM with PlatformIO:

```bash
cd firmware
pio run -t upload
```

The camera serves JPEG captures and an MJPEG stream protected by a bearer token. See [docs/firmware.md](docs/firmware.md).

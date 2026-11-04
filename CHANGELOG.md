# Changelog

## 2.0.0

A rewrite of the server and firmware to make the project usable by other people.

- Object detection with NanoDet: people and animals raise alerts, shadows and light changes do not. Three modes: off, alert on objects, only objects. Events carry labels, the best frame and its boxes; the interface filters by people, animals and shows clips without objects apart.
- Motion detection compensates global brightness drift (auto exposure, lamps).
- Frames of an event are written to disk while recording instead of held in memory; interrupted recordings are recovered at start; encoding runs one at a time at low priority.
- Password login next to passkeys; `camd-ctl password`.
- ntfy as an alert channel; Home Assistant MQTT discovery with a mode selector and privacy switch.
- Every setting can be given as `CAMD_<KEY>`; the server accepts requests from the host it is reached through, not only the configured origin.
- Interface in English and Spanish, site name and language configurable, dynamic web manifest.
- A resolution setting; the camera control API covers exposure, gain, white balance, saturation and sharpness.
- Firmware split into modules, with a captive setup page, mDNS, a watchdog, frame-stall recovery and constant-time token checks. The `?token=` query form was removed.
- Docker image, compose file, installer and hardened systemd unit; CI for the server, the interface, the firmware and the image.
- MQTT user, password and TLS; push endpoints must be public HTTPS; the camera setup page asks for the current token once the camera is configured; client addresses for rate limiting come from the proxy's `X-Forwarded-For`.
- API split into routers, database migrations tracked with `user_version`, 100+ tests including a full pipeline test with a fake camera.

## 1.x

Private versions: live view, events, photos, manual clips, passkeys, Web Push and the PWA.

# Changelog

## 3.1.1

- An object that was accepted keeps its own place active, so someone who walks in and then sits still keeps counting, and keeps the event open, for as long as they are seen. In 3.1.0 a person who barely moved stopped counting 10 seconds after their last big move and the clip ended with them still in the picture. An object that never moved is still never accepted.
- An object entirely inside an ignored zone does not count, since no motion is measured there (documented; it was already so in 3.1.0).
- Firmware 1.0.2: while someone is connected to the setup portal the board restarts after 30 minutes offline instead of never, so a phone that rejoins the open setup network by itself cannot keep the camera offline for good. The `portal` console command on a connected camera opens the portal without dropping the wifi, as before 1.0.1.

## 3.1.0

- An object only counts if something moved where it is. The motion detector now keeps a coarse map (20 by 15 cells) of where the picture changed in the last 10 seconds, and a detection whose box touches none of those cells is dropped before it labels an event, raises an alert, reaches MQTT or the live view. A backpack or a coat that the model sometimes takes for a person no longer turns a cat or a change of light elsewhere into a person event. Someone who walks in and then stays still keeps counting for 10 seconds after the last move, which is longer than the gap between two analysed frames during an event.
- Firmware 1.0.1: the setup portal can be seen. When the saved network is missing, the board stops retrying it in the background while the portal is open (the retries moved the access point from channel to channel, so phones never listed it); it tries the saved network again once a minute while nobody is connected to the portal, and no longer restarts after 10 minutes offline while someone is.

## 3.0.0

The server is rewritten in Rust. It does the same job with the same configuration, the same data directory and the same HTTP API, so upgrading is replacing the program, and going back to 2.2 is replacing it again. What changes is what it costs to run: camd is now one static binary of about 28 MB (22 MB on ARM64) with the web app inside, with no Python, OpenCV or OpenSSL to install. It needs only FFmpeg for clips and, for presence, `ping` and `arping`.

Measured on the same machine (a 2 vCPU Debian 13 container) with the same 800x600 stream at 10 fps, 3.0 against 2.2:

- CPU in a quiet room: 4.0 % of one core against 9.3 %; with continuous motion, detection and clips: 11.1 % against 17.6 %; motion analysis takes 3.7 ms per frame against 9.1 ms, with the same reduced JPEG decode and the same Gaussian blur as OpenCV, bit for bit;
- memory with the detector loaded: 36.8 MB against 131.3 MB; 13.6 MB after a fresh start, before the detector is first needed;
- memory after 10 minutes of continuous motion: 40.2 MB and flat, against 157.9 MB;
- one inference: about 27 ms against 33 ms;
- 7 threads against 13; answering HTTP 35 ms after start against 923 ms; stopping with a viewer connected takes 0.03 s against 5.2 s.

What stays the same:

- Every configuration key, default and `CAMD_*` variable, `CAMD_CONFIG`, and `[[cameras]]` tables.
- The data directory: `camd.sqlite3` with the same schema, `clips/`, `photos/`, `models/`, the spool files and `vapid_private.pem`. 2.2 opens a database written by 3.0.
- Passkeys, the password, sessions and cookies, enrollment codes and push subscriptions: nobody has to sign in or subscribe again.
- Every HTTP path, status code and JSON field, the origin checks and `trust_proxy`, MQTT topics and Home Assistant discovery, the Notifich, ntfy and Web Push payloads and the alert texts in English and Spanish.
- The pipeline: motion with brightness compensation, NanoDet-Plus-m on frames with motion at the same cadence and thresholds (the same file, now run with tract instead of OpenCV, giving the same boxes), the three detection modes, alert zones, pre-roll, the on-disk spool, one FFmpeg at a time with the same arguments, retention, arming by presence, snooze and privacy.
- `camd-ctl` becomes `camd ctl` (`password`, `enroll`, `devices`, `revoke-all`); the installer adds a `camd-ctl` link so old scripts keep working.

Fixes and deliberate changes:

- A value of the wrong type in the TOML file (`retention_days = "14"`) stops the server with a message naming the key; 2.2 accepted it silently. A `[[cameras]]` entry whose `id` is not a string, or that sets `camera_id`, is rejected.
- Events recovered after a crash keep the objects that were seen. 2.2 cleared them, so an event with a person was shown as "No object" and deleted after 24 hours.
- The detection model is dropped from memory when detection is turned off and loaded again when it is turned on.
- Turning the camera off, privacy, standby and stopping the server close the stream connection at once.
- If the spool file of a new event cannot be created, the event is removed instead of staying "recording" forever.
- A damaged JPEG from the camera is dropped and never stops a thread; header lines from the camera are limited in length; FFmpeg's error output is read while it runs and its timeout really applies.
- Snoozing takes effect at once instead of after up to a second.
- Each database migration step runs in one transaction. JSON stored in the database is written compactly (2.2 reads it).
- The build id shown in Settings, System is always a hash of the interface files; 2.2 said "dev" when it was not packaged.
- HTTP: request bodies are limited to 256 KiB (`413` above); `HEAD` is answered on every `GET` route instead of `405`; registering the same passkey twice or sending a push subscription with a malformed endpoint answers `400` instead of `500`; static files carry an `ETag` tied to the build and answer `304`, without `Last-Modified`; a request for several byte ranges gets the whole file.
- Passkeys are verified by a small built-in verifier that accepts `none` and `packed` attestation with ES256, EdDSA or RS256 keys. camd asks browsers for `none` attestation, as 2.2 did, so this covers the passkeys browsers create.
- MQTT over TLS checks the broker against the system certificates, with a bundled set as fallback.
- The `event N failed` log line ends with the reason instead of a traceback.
- Privacy holds even when the camera is unreachable at start. In 2.2, if privacy was on and the camera came up after the server (after a power cut, for example), the status said privacy while the live view and snapshots showed the room. Now the server marks the camera as in standby, does not stream, and puts it back in standby when it appears; live view, snapshots, photos and manual clips are refused while privacy is on, whatever the camera reports. A frame already in flight when privacy turns on can no longer open an event.
- Stopping the server closes and saves the open event first and waits at most 3 s for the other threads, so a stalled model download or a slow alert no longer delays shutdown.
- A manual clip that cannot start answers `500`, as in 2.2.
- With `trust_proxy = false` and a proxy on the same machine (`127.0.0.1`), the client address is taken from `X-Forwarded-For` as 2.2 did, so login limits stay per visitor.
- `/api/settings` returns whole numbers for settings written as whole numbers in the TOML file until they are changed, as 2.2 did.
- Hardening: images larger than 2592x2592 are refused before decoding; at most 2 password checks run at once and at most 60 attempts per minute are accepted across all clients (`429` above); at most 1024 passkey challenges are pending; request headers must arrive within 10 s and at most 256 connections are open; the rate limiter tracks at most 4096 addresses; Web Push never follows redirects and refuses endpoints whose name resolves to a private or loopback address; FFmpeg is fed from its own thread so its timeout covers the whole encode; corrupt spool files end instead of allocating; camera headers are limited to 100 lines; `camd ctl devices | head` no longer fails on the closed pipe.

The interface is redesigned. The live page puts the camera name and one status line above the picture, and the day's timeline and the latest events beside it; frame rate, signal and detector time sit quietly under the video. The sidebar shows the camera name, or the camera selector when there are several, with the same status wording as the page. Screenshots in the README are taken from the real server and a fake camera.

Packaging:

- Release files: `camd-linux-amd64` and `camd-linux-arm64` (static, musl), the firmware image, `install.sh`, `camd.service`, `config.example.toml` and `SHA256SUMS`.
- `install.sh` downloads the binary for the machine, checks it against `SHA256SUMS`, and installs or upgrades in place, backing up the unit, `/etc/camd` and the database first.
- The systemd unit runs `/usr/local/bin/camd` with the 2.x hardening and more: a 512 MB memory cap, a 15 s stop timeout, `RestrictAddressFamilies`, `ProtectKernelLogs`, `ProtectClock`, `ProtectHostname`, `RestrictRealtime`, `SystemCallArchitectures=native` and `UMask=0027`; the env file is optional.
- The Docker image is Alpine with FFmpeg, `ping`, `arping` and time zone data, built from the release binaries, for `linux/amd64` and `linux/arm64`, with the same `camd` user id as before. `docker-compose.yml` now pulls `ghcr.io/cristobaltormo/camd:3` instead of building.
- CI runs `cargo fmt`, `cargo clippy -D warnings`, `cargo test` with FFmpeg and a Mosquitto broker, `cargo audit` with no ignored advisories, the interface tests, and the browser tests against the real binary and a fake camera.
- Dependencies: the MQTT client is built in (MQTT 3.1.1 over TCP or rustls: last will, keepalive, QoS 0 retained states, command subscriptions, reconnect every 5 s) instead of `rumqttc`, which pulled an older `rustls-webpki` and the unmaintained `rustls-pemfile`. The RustCrypto crates move to the current generation (`p256` 0.14, `rsa` 0.10, `ed25519-dalek` 3, `sha2` 0.11, `aes-gcm` 0.11, `hkdf` 0.13, `scrypt` 0.12) with a single `digest`, which closes the timing advisories in `crypto-bigint` 0.5 and `rsa` 0.9. 303 crates in `Cargo.lock` instead of 316.

### Upgrading from 2.x

With the installer, run it again; it detects the Python install and keeps it:

```sh
curl -fsSL https://github.com/cristobaltormo/camd/releases/latest/download/install.sh | sudo bash
```

It stops the service, copies the unit, `/etc/camd`, the database and the push key to `/var/backups/camd/<date>-before-3.0.0/`, installs `/usr/local/bin/camd` and the new unit, and starts it again. `/etc/camd/config.toml`, `/etc/camd/env` and `/var/lib/camd` are used as they are. With Docker: `docker compose pull && docker compose up -d` (if your compose file still has `build: .`, replace it with `image: ghcr.io/cristobaltormo/camd:3`). Before upgrading, check that no value in `config.toml` has the wrong type.

### Going back to 2.2

The installer leaves `/opt/camd/venv` in place. Restore the old unit and start it:

```sh
sudo systemctl stop camd
sudo cp /var/backups/camd/<date>-before-3.0.0/camd.service /etc/systemd/system/camd.service
sudo systemctl daemon-reload && sudo systemctl start camd
```

2.2 reads what 3.0 wrote, so events, photos and sign-ins made in between are kept. With Docker, set the image back to `ghcr.io/cristobaltormo/camd:2.2.0`.

## 2.2.0

- Several cameras on one server: `[[cameras]]` tables in the configuration, one runtime and data directory each, a selector in the app, alerts that carry the camera name. See [configuration](docs/configuration.md#several-cameras).
- Alert zones: an object raises an alert only when the centre of its box is inside a zone drawn in Settings, Detection.
- A day view in Events with a 24 hour timeline coloured by what was seen; `GET /api/events` accepts `after`.
- CI runs `mypy` and enforces a coverage floor of 85%.

## 2.1.0

A new interface and the project files expected of a public repository.

- Interface redesigned: a restrained dark and light theme, Inter bundled with the app (no external requests), thinner icons, a status list instead of badges on the live page, a monospace overlay with clock and resolution, sharper detection boxes, quieter filters and settings panels. Brand mark and app icons redrawn.
- The player sizes itself to the image so detection boxes line up with the picture at any resolution.
- Screenshots and the social preview are generated by `make screenshots` and `make brand` from a real photograph and a real detection instead of drawings.
- Community files: code of conduct, issue and pull request templates, Dependabot, CodeQL, a Docker image for amd64 and arm64 on release, `.editorconfig`, pre-commit, a Makefile.
- Documentation: architecture, HTTP API, hardware, troubleshooting, FAQ; `camd --version`.

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

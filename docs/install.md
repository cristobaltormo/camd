# Installing the server

camd needs Python 3.11 or newer, FFmpeg and about 300 MB of RAM. It runs on x86-64 and ARM64.

## Docker

```bash
git clone https://github.com/OWNER/camd && cd camd
cp .env.example .env
```

Edit `.env`:

| Variable | Meaning |
|---|---|
| `CAMD_CAMERA_HOST` | address or `.local` name of the camera |
| `CAMD_CAMERA_TOKEN` | the token generated on the camera setup page |
| `CAMD_ORIGIN` | the address people will type, for example `http://192.168.1.20:8180` |
| `CAMD_RP_ID`, `CAMD_SECURE_COOKIES`, `CAMD_TRUST_PROXY` | only when you serve HTTPS behind a proxy (see [remote access](remote-access.md)) |
| `CAMD_PRESENCE_DEVICES` | comma separated phone IPs that mean "I am home" (optional) |

Any key of the [configuration](configuration.md) can be set as `CAMD_<KEY>`.

```bash
docker compose up -d --build
docker compose exec camd camd-ctl password     # create the login password
```

Add `--profile mqtt` to also start a Mosquitto broker for Home Assistant and set `CAMD_MQTT_HOST=mosquitto`. mDNS names such as `camd-1a2b3c.local` do not resolve inside containers, so use the camera's IP and reserve it in your router.

Data (database, clips, photos, the detection model) lives in the `camd-data` volume.

## Debian or Ubuntu

```bash
git clone https://github.com/OWNER/camd && cd camd
sudo deploy/install.sh
```

It creates a `camd` user, a virtualenv in `/opt/camd`, the configuration in `/etc/camd` and a hardened systemd unit. Then:

1. put the token in `/etc/camd/env` as `CAMD_CAMERA_TOKEN=...`;
2. edit `/etc/camd/config.toml`: `camera_host` and `origin`. The default is plain HTTP on the LAN; `deploy/config.https.example.toml` is the version for HTTPS behind a proxy;
3. create the login: `sudo -u camd CAMD_CONFIG=/etc/camd/config.toml /opt/camd/venv/bin/camd-ctl password`;
4. `sudo systemctl start camd` and read the log with `journalctl -u camd -f`.

To update, pull and run `sudo deploy/install.sh` again, then `systemctl restart camd`.

## Signing in

| Method | When it works |
|---|---|
| Password | everywhere, including plain HTTP on your LAN. `camd-ctl password` sets it. |
| Passkey | only over HTTPS (or `localhost`). Create a code with `camd-ctl enroll`, open the app, and register the device. |

If both exist the sign-in screen offers both. `camd-ctl devices` lists passkeys and sessions, `camd-ctl revoke-all` signs everyone out.

## HTTPS and a public address

Passkeys, Web Push and installing the app on a phone all need HTTPS. See [remote access](remote-access.md). In short, set `origin` and `rp_id` to the final address, `secure_cookies = true`, and put a reverse proxy in front. The ready-made `deploy/nginx.conf` and `deploy/Caddyfile` already handle the live stream (no buffering, long timeout).

## First start

The first time detection is enabled the server downloads the 3.8 MB model into the data directory. Watch **Settings, Detection** for the state of the detector; until it is ready the server records and alerts on every motion, so nothing is missed. On a machine without internet access, copy `nanodet-plus-m.onnx` into `<data_dir>/models/` yourself; the file name and checksum are in `server/camd/detect.py`.

## Backups

Everything worth keeping is under the data directory. Stop the service (or use SQLite's `.backup`) and copy it.

# Installing the server

camd is one static binary for Linux on x86-64 or ARM64, with the web app built in. Besides it you need FFmpeg (clips are encoded with it) and, for presence detection, `ping` and optionally `arping`. The server uses about 40 MB of memory with the detector loaded.

## The installer (systemd)

```sh
curl -fsSL https://github.com/cristobaltormo/camd/releases/latest/download/install.sh | sudo bash
```

or, from a clone, `sudo deploy/install.sh`. It:

- installs `ffmpeg`, `iputils-ping`, `iputils-arping` and `curl` with apt (on other distributions it checks they are there and tells you what is missing);
- downloads `camd-linux-amd64` or `camd-linux-arm64` from the release, checks it against `SHA256SUMS` and installs it as `/usr/local/bin/camd`, with a `camd-ctl` link;
- creates a `camd` system user, `/var/lib/camd` for the data and `/etc/camd` with an example `config.toml` and an `env` file for secrets, readable only by root and the `camd` group;
- installs and enables the hardened unit `camd.service`, but does not start it on a fresh install.

`--version v3.0.0` (or `CAMD_VERSION`) installs a given release instead of the latest; `--binary ./camd` (or `CAMD_BINARY`) installs a binary you built yourself. Then:

1. put the token in `/etc/camd/env` as `CAMD_CAMERA_TOKEN=...`;
2. edit `/etc/camd/config.toml`: `camera_host` and `origin`. The default is plain HTTP on the LAN; `deploy/config.https.example.toml` is the version for HTTPS behind a proxy;
3. create the login: `sudo -u camd CAMD_CONFIG=/etc/camd/config.toml camd ctl password`;
4. `sudo systemctl start camd` and read the log with `journalctl -u camd -f`.

The unit only lets camd write to `/var/lib/camd`. If you set another `data_dir`, add it with `systemctl edit camd`:

```ini
[Service]
ReadWritePaths=/srv/camd
```

### Upgrading

Run the installer again. It stops the service, copies the unit, `/etc/camd`, the binary, the database files and the push key to `/var/backups/camd/<date>-before-<version>/`, installs the new binary and unit, keeps your `config.toml` and `env`, starts the service again if it was running and prints how to go back. Old backups are never removed: delete them yourself when you no longer need them.

### Upgrading from 2.x (Python)

The same command. 3.0 reads the configuration, the environment file and the data directory of 2.x as they are: events, clips, photos, passkeys, the password, sessions and push subscriptions keep working, and nobody has to sign in again. The Python virtualenv in `/opt/camd/venv` is not touched. To go back to 2.x:

```sh
sudo systemctl stop camd
sudo cp /var/backups/camd/<date>-before-3.0.0/camd.service /etc/systemd/system/camd.service
sudo systemctl daemon-reload && sudo systemctl start camd
```

With Docker: `docker compose pull && docker compose up -d`; a 2.x compose file with `build: .` needs `image: ghcr.io/cristobaltormo/camd:3` instead. `camd-ctl` still works in the image and on systemd installs by name; a script that calls the old venv path `/opt/camd/venv/bin/camd-ctl` keeps running the 2.x one until it is changed to `camd ctl`.

2.x opens a database written by 3.0. When 3.0 has run for a while, remove the old server with `sudo rm -rf /opt/camd`. Before upgrading, check that no setting in `config.toml` has the wrong type (`retention_days = "14"` instead of `14`, for example): 3.0 refuses to start with a clear message where 2.x accepted it silently.

## Docker

```sh
git clone https://github.com/cristobaltormo/camd && cd camd
cp .env.example .env
```

Edit `.env`:

| Variable | Meaning |
|---|---|
| `CAMD_CAMERA_HOST` | address of the camera |
| `CAMD_CAMERA_TOKEN` | the token generated on the camera setup page |
| `CAMD_ORIGIN` | the address people will type, for example `http://192.168.1.20:8180` |
| `TZ` | your time zone, for example `Europe/Madrid`; quiet hours and the dates in the app use it |
| `CAMD_RP_ID`, `CAMD_SECURE_COOKIES`, `CAMD_TRUST_PROXY` | only when you serve HTTPS behind a proxy (see [remote access](remote-access.md)) |
| `CAMD_PRESENCE_DEVICES` | comma separated phone IPs that mean "I am home" (optional) |

Any key of the [configuration](configuration.md) can be set as `CAMD_<KEY>`.

```sh
docker compose up -d
docker compose exec camd camd ctl password     # create the login password
```

The compose file uses `ghcr.io/cristobaltormo/camd:3`, published for `linux/amd64` and `linux/arm64` with every release. The image is Alpine with FFmpeg, `ping`, `arping` and time zone data; it runs as the unprivileged user `camd` (uid 999, the same as the 2.x image, so an existing `camd-data` volume works as it is) and has a health check on `/healthz`. To build it yourself: `docker build -t camd .`.

Add `--profile mqtt` to also start a Mosquitto broker for Home Assistant and set `CAMD_MQTT_HOST=mosquitto`. mDNS names such as `camd-1a2b3c.local` do not resolve inside containers, so use the camera's IP and reserve it in your router. On Docker's default bridge network `arping` cannot see the phones on your LAN; presence then relies on `ping`, which is enough for most phones, or use `network_mode: host`.

Data (database, clips, photos, the detection model) lives in the `camd-data` volume.

## A release binary by hand

```sh
f=camd-linux-arm64
curl -fLO https://github.com/cristobaltormo/camd/releases/latest/download/$f
curl -fLO https://github.com/cristobaltormo/camd/releases/latest/download/SHA256SUMS
grep " $f\$" SHA256SUMS | sha256sum -c
sudo install -m 755 $f /usr/local/bin/camd
CAMD_CONFIG=./config.toml CAMD_DATA_DIR=./data camd
```

`deploy/camd.service` is the unit the installer uses, if you want to write your own setup.

## From source

With a current stable [Rust toolchain](https://rustup.rs):

```sh
git clone https://github.com/cristobaltormo/camd && cd camd
cargo build --release --locked
sudo deploy/install.sh --binary target/release/camd
```

The release build uses link-time optimisation and needs about 2 GB of memory to link. The static binaries of the releases are built with `make release-local` (see [development](development.md)).

## Signing in

| Method | When it works |
|---|---|
| Password | everywhere, including plain HTTP on your LAN. `camd ctl password` sets it. |
| Passkey | only over HTTPS (or `localhost`). Create a code with `camd ctl enroll`, open the app, and register the device. |

If both exist the sign-in screen offers both. `camd ctl devices` lists passkeys and sessions, `camd ctl revoke-all` signs everyone out. Run them as the user the service runs as and with the same configuration, so they open the same database: `sudo -u camd CAMD_CONFIG=/etc/camd/config.toml camd ctl devices`, or `docker compose exec camd camd ctl devices`.

## HTTPS and a public address

Passkeys, Web Push and installing the app on a phone all need HTTPS. See [remote access](remote-access.md). In short, set `origin` and `rp_id` to the final address, `secure_cookies = true`, and put a reverse proxy in front. The ready-made `deploy/nginx.conf` and `deploy/Caddyfile` already handle the live stream (no buffering, long timeout).

## First start

The first time detection is needed the server downloads the 3.8 MB model into `<data_dir>/models/` and checks its SHA-256. Watch **Settings, Detection** for the state of the detector; until it is ready the server records and alerts on every motion, so nothing is missed. On a machine without internet access, download `object_detection_nanodet_2022nov.onnx` from the [OpenCV Zoo](https://github.com/opencv/opencv_zoo/tree/main/models/object_detection_nanodet) yourself and copy it as `<data_dir>/models/nanodet-plus-m.onnx`; its SHA-256 must be `4b82da9944b88577175ee23a459dce2e26e6e4be573def65b1055dc2d9720186`.

## Backups

Everything worth keeping is under the data directory: `camd.sqlite3`, `clips/`, `photos/` and `vapid_private.pem` (the key push subscriptions are tied to). Stop the service, or use `sqlite3 camd.sqlite3 ".backup copy.sqlite3"` while it runs, and copy it together with `/etc/camd`.

# Configuration

Settings come from three places, in this order: built-in defaults, the TOML file (`/etc/camd/config.toml`, or the path in `CAMD_CONFIG`), and environment variables named `CAMD_<KEY>` (lists are comma separated, booleans accept `true`, `1`, `yes`, `on`). Unknown keys in the file are an error, so typos do not go unnoticed.

Keys marked `(live)` can also be changed from the app (Settings). A value saved there wins over both the file and the environment, survives restarts, and is the one in force from then on.

| Key | Default | What it does |
|---|---|---|
| `camera_host` | `""` | Address or name of the ESP32-CAM. Required. |
| `camera_api_port` | `80` | Port of the camera control API. |
| `camera_stream_port` | `81` | Port of the camera MJPEG stream. |
| `camera_token` | `""` | Access token set on the camera. Pass it as `CAMD_CAMERA_TOKEN`. |
| `data_dir` | `"/var/lib/camd"` | Where the database, clips, photos and models live. |
| `listen_host` | `"127.0.0.1"` | Interface the server binds to. Keep `127.0.0.1` behind a reverse proxy. |
| `listen_port` | `8180` | Port the server listens on. |
| `site_name` | `"Room camera"` | Name shown in the app, the installed icon and alerts. |
| `language` (live) | `"en"` | Language of alert texts (`en` or `es`). The interface follows the browser. |
| `rp_id` | `"localhost"` | Domain used by passkeys. Must be the host name people type, without scheme or port. |
| `origin` | `"http://localhost:8180"` | Public address of the app, with scheme and port. Passkeys are verified against it. |
| `secure_cookies` | `false` | Mark the session cookie `Secure`. Turn on whenever you serve HTTPS. |
| `trust_proxy` | `false` | Read the client address from `X-Forwarded-For` or `CF-Connecting-IP` (rate limits). |
| `push_subject` | `"https://localhost"` | Contact URL sent with Web Push messages (`mailto:` or `https:`). |
| `notifich_url` | `""` | Base URL of a Notifich server (optional channel). |
| `notifich_key` | `""` | Notifich API key (`CAMD_NOTIFICH_KEY`). |
| `ntfy_url` | `""` | Full topic URL on an ntfy server, for example `https://ntfy.sh/my-secret-topic` (optional channel). |
| `ntfy_token` | `""` | ntfy access token for protected topics (`CAMD_NTFY_TOKEN`). |
| `mqtt_host` | `""` | MQTT broker. Empty disables MQTT. |
| `mqtt_port` | `1883` | MQTT broker port. |
| `mqtt_user` | `""` | MQTT user name (optional). |
| `mqtt_password` | `""` | MQTT password (`CAMD_MQTT_PASSWORD`). |
| `mqtt_tls` | `false` | Use TLS to the broker. |
| `mqtt_topic` | `"camd"` | Topic prefix for state and commands. |
| `mqtt_discovery` | `true` | Announce the entities to Home Assistant automatically. |
| `presence_devices` | `[]` | IP addresses of phones that mean someone is home. Empty means manual arming only. |
| `presence_grace_s` (live) | `300.0` | Seconds without an answer before a phone counts as away. |
| `retention_days` (live) | `14` | Days to keep events (protected ones are never deleted). |
| `max_disk_gb` (live) | `6.0` | Space cap for events. The oldest go first. |
| `pre_roll_s` (live) | `3.0` | Seconds recorded before motion starts. |
| `post_roll_s` (live) | `10.0` | Seconds without activity before a clip is closed. |
| `max_event_s` (live) | `90.0` | Longest single clip; longer activity is split. |
| `push_cooldown_s` (live) | `60.0` | Minimum seconds between two alerts. |
| `notify_motion` (live) | `true` | Send detection alerts. |
| `notify_offline` (live) | `true` | Alert when the camera stops sending images. |
| `quiet_enabled` (live) | `false` | Mute detection alerts during quiet hours. |
| `quiet_start` (live) | `"23:00"` | Start of quiet hours (`HH:MM`). |
| `quiet_end` (live) | `"07:00"` | End of quiet hours (`HH:MM`). |
| `detection` (live) | `"alert"` | `off` records and alerts on any motion; `alert` records everything but alerts only on objects; `strict` keeps only clips with objects. |
| `detect_labels` (live) | `["person", "cat", "dog"]` | Objects that count: any of the 80 COCO names, for example `person`, `cat`, `dog`, `car`. |
| `detect_confidence` (live) | `0.5` | Minimum confidence (0.2 to 0.9). |
| `unconfirmed_keep_h` (live) | `24.0` | Hours to keep clips without objects in `alert` mode. |
| `camera_enabled` (live) | `true` | Turn off before unplugging the camera so nothing alerts. |
| `camera_vflip` (live) | `false` | Flip the image vertically. |
| `camera_hmirror` (live) | `false` | Mirror the image horizontally. |
| `camera_brightness` (live) | `0` | Brightness, -2 to 2. |
| `camera_contrast` (live) | `0` | Contrast, -2 to 2. |
| `camera_quality` (live) | `0` | JPEG quality, 4 (best) to 63. 0 keeps the camera default. |
| `camera_framesize` (live) | `0` | Resolution code: 5 = 320x240, 8 = 640x480, 9 = 800x600, 10 = 1024x768. 0 keeps the camera default. |

## Several cameras

One server can watch more than one ESP32-CAM. Add a `[[cameras]]` table per extra camera to the configuration file; the top-level keys describe the first camera and are the defaults for the rest.

```toml
camera_host = "192.168.1.50"
site_name = "Home"

[[cameras]]
id = "porch"
name = "Porch"
camera_host = "192.168.1.51"
camera_token = "0123456789abcdef01234567"
```

Only keys listed above are accepted inside a table, plus `id` (lowercase letters, digits and dashes, unique) and `name` (what the app shows). Each extra camera gets its own data directory (`<data_dir>/cameras/<id>` unless you set `data_dir`), so events, photos, settings and the detection state are independent; sign-in, alert channels and the push subscriptions are shared. The MQTT topic defaults to `<mqtt_topic>_<id>` and alerts carry the camera name. Each camera needs its own token: set it as `camera_token` in its table.

The app shows a camera selector when there is more than one. The selection is a cookie (`cam_camera`), so it applies to every tab of the same browser.

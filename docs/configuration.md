# Configuration

Settings come from three places, in this order: built-in defaults, the TOML file (`/etc/camd/config.toml`, or the path in `CAMD_CONFIG`), and environment variables named `CAMD_<KEY>` (lists are comma separated, booleans accept `true`, `1`, `yes`, `on`). Unknown keys in the file are an error, so typos do not go unnoticed.

Keys marked with a star can also be changed from the app (Settings); the value saved there wins over the file and survives restarts.

| Key | Default | What it does |
|---|---|---|
| `camera_host` | `"192.168.1.93"` | Address or name of the ESP32-CAM. |
| `camera_api_port` | `80` | Port of the camera control API. |
| `camera_stream_port` | `81` | Port of the camera MJPEG stream. |
| `camera_token` | `""` | Access token set on the camera. Pass it as `CAMD_CAMERA_TOKEN`. |
| `data_dir` | `"/var/lib/camd"` | Where the database, clips, photos and models live. |
| `listen_host` | `"127.0.0.1"` | Interface the server binds to. Keep `127.0.0.1` behind a reverse proxy. |
| `listen_port` | `8180` | Port the server listens on. |
| `site_name` | `"Room camera"` | Name shown in the app, the installed icon and alerts. |
| `language` ★ | `"en"` | Language of alert texts (`en` or `es`). The interface follows the browser. |
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
| `mqtt_topic` | `"camd"` | Topic prefix for state and commands. |
| `mqtt_discovery` | `true` | Announce the entities to Home Assistant automatically. |
| `presence_devices` | `[]` | IP addresses of phones that mean someone is home. Empty means manual arming only. |
| `presence_grace_s` ★ | `300.0` | Seconds without an answer before a phone counts as away. |
| `retention_days` ★ | `14` | Days to keep events (protected ones are never deleted). |
| `max_disk_gb` ★ | `6.0` | Space cap for events. The oldest go first. |
| `pre_roll_s` ★ | `3.0` | Seconds recorded before motion starts. |
| `post_roll_s` ★ | `10.0` | Seconds without activity before a clip is closed. |
| `max_event_s` ★ | `90.0` | Longest single clip; longer activity is split. |
| `push_cooldown_s` ★ | `60.0` | Minimum seconds between two alerts. |
| `notify_motion` ★ | `true` | Send detection alerts. |
| `notify_offline` ★ | `true` | Alert when the camera stops sending images. |
| `quiet_enabled` ★ | `false` | Mute detection alerts during quiet hours. |
| `quiet_start` ★ | `"23:00"` | Start of quiet hours (`HH:MM`). |
| `quiet_end` ★ | `"07:00"` | End of quiet hours (`HH:MM`). |
| `detection` ★ | `"alert"` | `off` records and alerts on any motion; `alert` records everything but alerts only on objects; `strict` keeps only clips with objects. |
| `detect_labels` ★ | `["person", "cat", "dog"]` | Objects that count: any of the 80 COCO names, for example `person`, `cat`, `dog`, `car`. |
| `detect_confidence` ★ | `0.5` | Minimum confidence (0.2 to 0.9). |
| `unconfirmed_keep_h` ★ | `24.0` | Hours to keep clips without objects in `alert` mode. |
| `camera_enabled` ★ | `true` | Turn off before unplugging the camera so nothing alerts. |
| `camera_vflip` ★ | `false` | Flip the image vertically. |
| `camera_hmirror` ★ | `false` | Mirror the image horizontally. |
| `camera_brightness` ★ | `0` | Brightness, -2 to 2. |
| `camera_contrast` ★ | `0` | Contrast, -2 to 2. |
| `camera_quality` ★ | `0` | JPEG quality, 4 (best) to 63. 0 keeps the camera default. |
| `camera_framesize` ★ | `0` | Resolution code: 5 = 320x240, 8 = 640x480, 9 = 800x600, 10 = 1024x768. 0 keeps the camera default. |

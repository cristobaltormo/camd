# Architecture

```
                 MJPEG over HTTP, bearer token
 ESP32-CAM  ------------------------------------>  grabber  (one thread, one connection)
                                                      |
                                       +--------------+---------------+
                                       |                              |
                                  viewers hub                     motion detector
                              (fan-out to browsers)          (quarter-size gray frames,
                                                              background subtraction)
                                                                      |
                                                              object detector worker
                                                          (NanoDet, only on moving frames)
                                                                      |
                                                              event recorder
                                                  (pre-roll ring, spool on disk, FFmpeg)
                                                                      |
                                       +------------------+-----------+----------+
                                       |                  |                      |
                                    SQLite           alert channels        MQTT discovery
                              (events, photos,       (Web Push, ntfy,       (Home Assistant)
                               sessions, prefs)        Notifich)
```

With several cameras the service builds one runtime per camera (own database, grabber, detector and recorder) and shares the sign-in and the alert channels. The `cam_camera` cookie picks which runtime a request talks to.

## Why it is built this way

**The camera has one client.** The ESP32 serves a single stream connection, so the server reads it once and shares the frames with every browser. Opening the camera's address from a second place would steal the stream.

**Cheap first, expensive only when needed.** Motion detection works on small gray frames and costs a few percent of one core. The neural network only runs on frames where motion was found, at most every 0.7 seconds, so a quiet room costs almost nothing.

**Disk instead of memory.** The frames of an event are appended to a spool file while it is recorded and encoded by FFmpeg afterwards, one clip at a time and at low priority. Memory stays flat however long an event lasts, and a crash loses at most the last few frames: interrupted events are recovered when the service starts.

**Detection can fail without losing events.** If the model cannot be loaded the service behaves as if detection were off and alerts on every motion. A missing alert is worse than a false one.

**No build step for the interface.** The web app is plain ES modules. `build.rs` embeds them in the binary and adds a content hash to each asset URL, so a browser never mixes old and new files and there is nothing to deploy next to the binary.

**Few threads, nothing per frame that can wait.** HTTP and the live view run on a small async runtime with two worker threads. The grabber, the detector and the maintenance task have a thread each; SQLite, password hashing and FFmpeg run on blocking threads or as a child process. Motion is computed on a reduced grayscale decode of each JPEG, never on full-size colour frames, and the detection model is loaded only when detection is on and dropped from memory when it is turned off.

**One static binary.** No interpreter, no OpenCV, no OpenSSL: TLS is rustls, inference is [tract](https://github.com/sonos/tract) running the ONNX file, and SQLite is bundled. The release binaries are linked statically against musl, so the same file runs on any Linux of the same architecture.

## Modules

| Module | Responsibility |
|---|---|
| `grabber.rs` | reads the camera stream, reconnects, reports health, sends camera settings |
| `imaging.rs` | JPEG decode at reduced scale, resize, blur, encode |
| `motion.rs` | brightness-compensated background subtraction, ignored zones |
| `detect.rs`, `labels.rs` | NanoDet with tract, model download and checksum, worker thread, COCO names |
| `events.rs`, `spool.rs` | event lifecycle, pre-roll, on-disk frame spool, FFmpeg encoding |
| `photos.rs` | stills and their thumbnails |
| `arming.rs`, `presence.rs` | auto, armed and disarmed modes, snooze, privacy; phone presence by ping and ARP, plus passive listening for the phone's own frames |
| `sniff.rs` | packet socket with a kernel filter that passes only IPv4 and ARP frames sent by `presence_devices` |
| `viewers.rs` | live fan-out and per-viewer back-pressure |
| `notify.rs`, `push.rs`, `mqtt.rs` (`mqtt/`), `messages.rs` | alert channels, Web Push encryption and VAPID, Home Assistant discovery and the built-in MQTT 3.1.1 client, alert texts |
| `auth/` | passkeys (WebAuthn), password (scrypt), sessions, rate limits |
| `db/`, `storage.rs`, `prefs.rs` | SQLite schema and migrations, retention and size cap, settings saved from the app |
| `config.rs` | defaults, the TOML file, `CAMD_*` variables, several cameras |
| `runtime.rs` | wires one camera together and builds the status document |
| `app.rs`, `api/`, `assets.rs` | HTTP server, routes, origin checks, the embedded app |
| `ctl.rs` | `camd ctl` |

## Data directory

```
camd.sqlite3        events, photos, sessions, credentials, push subscriptions, settings
clips/              clips and thumbnails of events
photos/             stills
spool/              frames of events being recorded
models/             the detection model
vapid_private.pem   the Web Push key; subscriptions are tied to it
cameras/<id>/       the same layout for each extra camera
```

The layout and the database schema are the same as in 2.x, which is why 2.x and 3.0 can be swapped in both directions.

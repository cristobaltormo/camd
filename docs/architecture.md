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

## Why it is built this way

**The camera has one client.** The ESP32 serves a single stream connection, so the server reads it once and shares the frames with every browser. Opening the camera's address from a second place would steal the stream.

**Cheap first, expensive only when needed.** Motion detection works on small gray frames and costs a few percent of one core. The neural network only runs on frames where motion was found, at most every 0.7 seconds, so a quiet room costs almost nothing.

**Disk instead of memory.** The frames of an event are appended to a spool file while it is recorded and encoded by FFmpeg afterwards, one clip at a time and at low priority. Memory stays flat however long an event lasts, and a crash loses at most the last few frames: interrupted events are recovered when the service starts.

**Detection can fail without losing events.** If the model cannot be loaded the service behaves as if detection were off and alerts on every motion. A missing alert is worse than a false one.

**No build step for the interface.** The web app is plain ES modules served as they are. `server/stamp.py` adds a content hash to each asset URL when packaging, so a browser never mixes old and new files.

## Modules

| Module | Responsibility |
|---|---|
| `grabber.py` | reads the camera stream, reconnects, reports health |
| `motion.py` | brightness-compensated background subtraction, ignored zones |
| `detect.py` | NanoDet on OpenCV DNN, model download and checksum, worker thread |
| `events.py`, `spool.py` | event lifecycle, pre-roll, on-disk frame spool, FFmpeg encoding |
| `arming.py`, `presence.py` | auto, armed and disarmed modes; phone presence by ping and ARP |
| `viewers.py` | live fan-out and per-viewer back-pressure |
| `push.py`, `mqtt.py`, `messages.py` | alert channels and Home Assistant discovery |
| `auth.py` | passkeys, password, sessions, CSRF origin checks |
| `storage.py`, `db.py` | retention, size cap, SQLite migrations |
| `runtime.py` | wires everything together and builds the status document |
| `api/` | HTTP routers |
| `web/` | the app |

## Data directory

```
camd.db          events, photos, sessions, credentials, settings
events/          one folder per event: clip, thumbnail
photos/          stills
spool/           frames of events being recorded
models/          the detection model
```

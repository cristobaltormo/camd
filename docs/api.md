# HTTP API

The app is a client of this API, and so can your scripts be. Every endpoint except `/healthz`, `/manifest.webmanifest` and the sign-in ones needs a session cookie. Writes must come from the configured origin. The OpenAPI schema is not exposed.

## Public

| Method and path | Purpose |
|---|---|
| `GET /healthz` | liveness probe, used by the Docker health check |
| `GET /manifest.webmanifest` | web app manifest with the site name |
| `GET /api/auth/state` | whether you are signed in, which sign-in methods exist, site name and language |

## Sign-in

| Method and path | Purpose |
|---|---|
| `POST /api/auth/login/password` | `{password}`, sets the session cookie |
| `POST /api/auth/login/options`, `/login/verify` | passkey sign-in (WebAuthn) |
| `POST /api/auth/register/options`, `/register/verify` | register a device with an enrollment code |
| `POST /api/auth/logout` | end this session |
| `POST /api/auth/enroll-code` | create a ten minute enrollment code |
| `GET /api/auth/devices` | passkeys and sessions |
| `DELETE /api/auth/sessions/{id}`, `/credentials/{id}` | revoke one |

Sign-in endpoints are limited to ten attempts per minute per address.

## State and control

| Method and path | Purpose |
|---|---|
| `GET /api/cameras` | the configured cameras; the one used for the other calls is chosen with the `cam_camera` cookie |
| `GET /api/status` | camera, detector, armed state, current detections, last event, storage |
| `POST /api/mode` | `{mode: "auto" \| "armed" \| "disarmed"}` |
| `POST /api/snooze` | `{minutes}`, 0 resumes |
| `POST /api/privacy` | `{on}`, turns the camera sensor off or on |
| `GET /api/settings`, `POST /api/settings` | every setting; POST accepts any subset |

## Video and stills

| Method and path | Purpose |
|---|---|
| `GET /api/live` | MJPEG stream, shared between viewers |
| `GET /api/snapshot` | one JPEG |
| `POST /api/clips/start`, `/clips/stop` | manual clip, `{seconds}` |
| `GET /api/photos`, `POST /api/photos` | list, take a photo |
| `GET /api/photos/{id}/img`, `/thumb` | the image and its thumbnail |
| `DELETE /api/photos/{id}`, `POST /api/photos/delete` | delete one or many (`{ids}`) |

## Events

| Method and path | Purpose |
|---|---|
| `GET /api/events` | `limit`, `before`, `label`, `hide_unconfirmed` |
| `GET /api/events/count` | events since a timestamp, for the badge |
| `GET /api/events/{id}/clip`, `/thumb` | the MP4 and the thumbnail |
| `POST /api/events/{id}/keep` | `{keep}` protects an event from retention |
| `DELETE /api/events/{id}`, `POST /api/events/delete` | delete one or many |

## Alerts

| Method and path | Purpose |
|---|---|
| `GET /api/push/key` | the VAPID public key |
| `POST /api/push/subscribe`, `/unsubscribe` | register a browser |
| `POST /api/push/test` | send a test alert to every channel |

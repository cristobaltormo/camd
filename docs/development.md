# Development

```
Cargo.toml, build.rs  the server: one crate, one binary
src/                  the service
  api/                HTTP handlers: auth, control, events, media, notifications
  auth/               passkeys, password, sessions, rate limits
  db/                 SQLite schema, migrations and queries
web/                  the app: ES modules without a build step, embedded in the binary
tests/                integration tests (Rust) and fixtures written by the 2.x server
tests/web/            Node tests for the interface helpers and the translation catalog
firmware/             ESP32-CAM firmware (PlatformIO, Arduino)
tools/                fake camera, browser tests, screenshots, real-camera checks
deploy/               installer, systemd unit, proxy examples, example configs (the Dockerfile is at the root)
```

## Building and testing

You need a current stable [Rust toolchain](https://rustup.rs), FFmpeg (the tests encode real clips), and Node 22 for the interface tests.

```sh
make test     # cargo fmt --check, cargo clippy -D warnings, cargo test, node --test in tests/web
make run      # the server with deploy/config.example.toml and ./.data, on 127.0.0.1:8180
make build    # release build, copied to ./camd
```

`cargo test` covers every module, the HTTP API against a fake runtime, a pipeline test that streams real JPEG frames through the grabber, motion detector and recorder and checks the clip FFmpeg produces, a test that runs the real binary against a fake camera, and compatibility tests that open a database written by the 2.x server and check that 2.x can read what 3.0 writes. The detector is faked in tests, so nothing needs the network. The MQTT client is also tested against a real Mosquitto broker when one is installed at `/usr/sbin/mosquitto` (or `CAMD_MOSQUITTO` points to the binary); without it that test is skipped.

Run the tests in the dev profile. The release profile uses fat link-time optimisation: linking takes about 2 GB of memory, and `cargo test --release` can be killed for lack of memory on a machine with 3 GB.

## The interface

```sh
make ui       # cargo build, then tools/ui-test.sh test
```

`tools/ui-test.sh` starts the real binary (the newest of `target/debug/camd` and `target/release/camd`, or `CAMD_BIN`) with a fresh data directory, a fake camera that plays `docs/images/scene.jpg`, seeds it through the API and runs `tools/ui-smoke.mjs` and `tools/ui-e2e.mjs` in headless Chromium (set `CHROMIUM` to the binary if it is not called `chromium`). `tools/ui-test.sh serve` leaves everything running and prints the address and the password. The interface is embedded at build time, so rebuild after editing `web/`: the script refuses to run against a binary older than the files in `web/`.

The app has no build step. `build.rs` embeds `web/` and stamps every asset URL with a hash of the files (`?v=<build>`), as well as the service worker cache name and the `BUILD` constant, so a browser can never mix old and new files. The sources in git carry no stamps.

## A camera without hardware

```sh
mkdir frames
ffmpeg -f lavfi -i testsrc=size=800x600:rate=10 -frames:v 100 frames/%03d.jpg
python3 tools/fake_camera.py frames --token test --fps 10 --api-port 8081 --stream-port 8082
```

Point a camd at it with `camera_host = "127.0.0.1"`, `camera_api_port = 8081`, `camera_stream_port = 8082`, `camera_token = "test"`. `fake_camera.py` only needs Python's standard library.

## Screenshots and brand images

`make screenshots` builds the server and runs `tools/ui-test.sh screenshots`, which captures desktop and phone views in dark and light into `docs/images/`. Put the detection model in `~/.cache/camd-model/nanodet-plus-m.onnx` (or set `CAMD_MODEL`) so the person in the photograph is detected by the real model.

`make brand` (`tools/render-brand.sh`) renders the app icons from `web/icon.svg` and the social preview from `tools/brand/social.html`.

## Translations

User-facing text uses English sentences as keys: `t("Delete event")`. Spanish lives in `web/js/es.js`. `node --test` fails when a string has no translation, when a translation is no longer used, or when an object name of the detector has none. To add a language, create a catalog like `es.js`, register it in `text.js` and add its name to `LANGUAGES`; server-side alert texts are in `src/messages.rs`.

## Releases

A tag `vX.Y.Z` runs `.github/workflows/release.yml`: lint and the Rust tests (the interface, browser and audit checks run on every push to `main`), static musl binaries for x86-64 and ARM64 built with [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild), the firmware image, `SHA256SUMS` over everything, a GitHub release whose notes are the `## X.Y.Z` section of `CHANGELOG.md`, the web installer on GitHub Pages, and the multi-arch image on GHCR built from those same binaries (`Dockerfile`, target `prebuilt`).

To build the binaries locally:

```sh
cargo install --locked cargo-zigbuild     # and zig 0.14 on the PATH
rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
make release-local                        # dist/camd-linux-amd64, dist/camd-linux-arm64, SHA256SUMS
make platform PLATFORM=linux-arm64        # just one
```

The bundled SQLite does not link against musl with its large file wrappers, so `.cargo/config.toml` builds it with `-DSQLITE_DISABLE_LFS` (`CFLAGS_<target>`) for both musl targets; on 64-bit targets file offsets are 64 bits anyway. Without zig, a native x86-64 musl build works with `ZIGBUILD="cargo build" CC_x86_64_unknown_linux_musl=musl-gcc make platform PLATFORM=linux-amd64`.

## Checks against a real camera

Stop the service first: the camera serves one stream client at a time.

```sh
curl -H "Authorization: Bearer $TOKEN" http://CAMERA/status
curl -H "Authorization: Bearer $TOKEN" -o still.jpg http://CAMERA/capture
```

Then run the server in the foreground against it: the log says `stream connected` each time the grabber gets the stream, and the camera state, frame rate and signal are in `GET /api/status` and in Settings, System.

## Conventions

- Code and identifiers in English. No comments: names carry the intent and the reasoning belongs in the commit message or the docs.
- No emojis anywhere; icons are inline SVG.
- Interface text is never hard-coded in one language.
- Every behaviour change comes with a test.
- Keep the server light: no work per frame that can be done per event, and never hold the frames of an event in memory.

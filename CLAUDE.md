# camd

Self-hosted ESP32-CAM server: live view, motion and object detection, clips, alerts. One Rust binary (`src/`) with the web app (`web/`) embedded at build time, plus the ESP32 firmware (`firmware/`). Read `docs/architecture.md` for how it fits together and `docs/development.md` for the layout.

## Rules for this repo

- Code and identifiers in English. No comments in code or config: names carry the intent, reasoning goes in commit messages and docs.
- No emojis anywhere. Icons are inline SVG.
- Interface text goes through `t("English sentence")`; add the Spanish in `web/js/es.js`. Server-side alert text lives in `src/messages.rs`.
- Every behaviour change has a test and a line in `CHANGELOG.md`. Keep the server light: no work per frame that can be done per event, no frames kept in memory for the length of an event.
- The HTTP API, the configuration keys, the data directory and the database schema are a contract with 2.x installs (which can be swapped back): change them only on purpose and say so in the changelog.
- No secrets in files, commits or messages.

## Commands

| Task | Command |
|---|---|
| Everything CI checks except the browser | `make test` (fmt, clippy `-D warnings`, `cargo test`, `node --test` in `tests/web`) |
| Rust tests only | `cargo test` (needs ffmpeg) |
| Format | `cargo fmt` |
| Interface unit tests | `cd tests/web && node --test` |
| Browser tests | `cargo build && tools/ui-test.sh` (real binary, fake camera, headless Chromium) |
| Run against a fake camera, interactive | `tools/ui-test.sh serve` |
| Run the server | `make run` (example config, data in `./.data`) or `CAMD_CONFIG=... cargo run` |
| Fake camera | `python3 tools/fake_camera.py FRAMES_DIR --token test` |
| Static release binaries | `make release-local` (cargo-zigbuild and zig 0.14) |
| Screenshots, icons | `make screenshots`, `make brand` |
| Firmware build | `cd firmware && pio run` |

## Frontend

`web/` has no build step: ES modules in `js/` (`main.js` shell and hash router, one module per section), `theme.js` applies the saved theme before first paint, `sw.js` caches the shell network-first. The CSP forbids inline styles and scripts: set styles with `el.style.x = ...`, never `setAttribute("style")` or `<style>`. Sources carry no cache stamps; `build.rs` stamps them when it embeds `web/`, so any change in `web/` needs a rebuild before the browser tests (`tools/ui-test.sh` refuses a stale binary).

## Traps

- The camera serves one stream client at a time. Never open a second consumer while the service runs.
- Opening the board's serial port resets it; wait about 15 s before asking for `status`.
- Passkeys need a secure origin and a matching `rp_id`.
- The ESP32 only joins 2.4 GHz.
- The detection model is downloaded on first use; tests never need the network (the detector is faked with `detect::FakeDetector`).
- NanoDet outputs are identified by shape, and anchors carry `+stride/2`: without it boxes shift half a cell.
- Static musl builds of the bundled SQLite fail to link on the `*64` file functions: build with `CFLAGS_<target>=-DSQLITE_DISABLE_LFS` (the Makefile, Dockerfile and workflows set it).
- The release profile is fat LTO: linking peaks near 2 GB, and `cargo test --release` gets OOM-killed in a 3 GB machine. Test in the dev profile; `target/` grows by hundreds of MB per test binary, `cargo clean -p camd` frees it.
- Node 22: `node --test some/dir/` does not find the tests; run `node --test` inside `tests/web`.
- Chrome on the CI runner is slow to start: `tools/cdp.mjs` waits up to 40 s and passes `--disable-dev-shm-usage`. The runner's apt sometimes hangs: every apt step has retries and a timeout.
- CodeQL and GitHub Pages need the repository to be public: those jobs have `if: !github.event.repository.private`.
- `.sh` files must be executable in git (`git update-index --chmod=+x`), or CI fails with "permission denied".
- The Docker image keeps uid 999 for `camd` so 2.x volumes work: Alpine's `ping` group (gid 999) is removed for that.

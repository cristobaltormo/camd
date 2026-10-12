# habitacion-cam

Camera for Cristobal's room. Read `README.md` for the architecture. Working rules for this repo:

- Code and identifiers in English, talk to him in Spanish. No emojis anywhere (UI icons are inline SVG).
- Few comments: only the why of a trap. No secrets in files, commits or messages (Vaultwarden: `habitacion-cam token`,
  `wifi casa movistar (2.4GHz)`).
- Develop on the Proxmox host in `/root/proyectos/habitacion-cam`; the service runs in LXC 131 (`cam`, 192.168.1.73).
  Publish from the host with the `publicar-git` skill (signed as him, no Claude trailers). Never commit inside the LXC.
- It is exposed through its own Cloudflare tunnel with Access in front (`docs/cloudflare.md`, authorised by him on 2026-10-12).
  Do not widen that: no extra Access policies, no routing it through the shared tunnel or the VPS failover.
- Back up (vzdump of LXC 131 with date and reason) before changing anything that is already in use.

## Commands

| Task | Command |
|---|---|
| Run the tests on the LXC | `server/deploy.sh test` |
| Deploy and restart | `server/deploy.sh restart` |
| Service logs | `pct exec 131 -- journalctl -u habitacion-cam -f` |
| Passkey enrollment code (10 min) | `pct exec 131 -- sudo -u camd /opt/habitacion-cam/venv/bin/python -m camd.ctl enroll` |
| Revoke every session | `... -m camd.ctl revoke-all` |
| Camera status | `curl -H "Authorization: Bearer $TOKEN" http://192.168.1.93/status` |
| Firmware over the air | `pio run -e esp32cam-ota -t upload` with `CAM_TOKEN` exported |

## Frontend

`server/camd/web/` is a build-free PWA: ES modules in `js/` (`main.js` shell and hash router, one module per section), `theme.js` applies
the saved theme before first paint, `sw.js` caches the shell network-first. The CSP forbids inline styles and scripts: set styles through
`el.style.x = ...`, never `setAttribute("style")` or `<style>`. Bump `CACHE` in `sw.js`, `VERSION` in `server/camd/app.py` and the `?v=` of the three assets in `index.html` on UI releases (stale CSS with new JS shows giant icons).
To look at it without the camera run `python tools/ui-mock.py` and screenshot with `chromium --headless --screenshot` at 390x844 and 1440x900.

## Traps

- The camera serves one stream client at a time (its second HTTP server is single-threaded). Never open a second
  consumer while the service runs: stop `habitacion-cam` before `smoke.py`, `soak.py` or `diag.py`.
- Opening the board's serial port resets it; wait about 15 s before asking for `status`.
- `threading.Thread` owns `_stop`: name the stop flag differently.
- Passkeys need a secure origin and a matching `rp_id`: `cam.cristobal.cc`, never the raw LXC IP.
- The cristobal.cc zone has wildcard TXT/A records: DNS-01 checks must use public resolvers (see `tools/renew-cert.sh`).
- The ESP32 only joins 2.4 GHz. With the external antenna fitted the signal went from -80 to -50 dBm.

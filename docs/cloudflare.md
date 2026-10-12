# Outside access through Cloudflare (live since 2026-10-12)

```
phone -> Cloudflare edge -> Access (email one-time PIN, only his address, country restriction)
      -> tunnel "habitacion-cam" (dedicated, runs in LXC 131) -> http://127.0.0.1:8180 -> passkey login
```

| Piece | Where |
|---|---|
| Access app `habitacion-cam` (`cam.cristobal.cc`, 168 h session) | Cloudflare account `cristobal.cc`, Zero Trust. Policy `solo-cristobal` is the Proxmox one: his Gmail plus the `@cristobal.cc` domain (safe because the only mailboxes there are his and the catch-all goes to `admin@`; revisit if anyone else ever gets a mailbox on that domain) |
| Tunnel `habitacion-cam` (`a8af63be-...`, remotely managed) | Same account. Ingress `cam.cristobal.cc -> http://127.0.0.1:8180`, then 404 |
| DNS | Explicit proxied CNAME `cam` to the tunnel; it overrides the wildcard that points at the shared `cristobal.cc` tunnel |
| `cloudflared` | `/usr/local/bin/cloudflared` in the LXC (copy of the host binary), unit `habitacion-cam-tunnel`, token in `/etc/habitacion-cam/cloudflared.env` (root, 0600), `--no-autoupdate` |

The shared `cristobal.cc` tunnel was deliberately not touched, so nothing else can break because of the camera.

Two independent gates stand in front of the data: Access, then the passkey. The origin only listens on loopback and the
tunnel is outbound-only, so nothing is open on the router.

## Operations

- Tunnel down: `pct exec 131 -- journalctl -u habitacion-cam-tunnel -n 50`. Update `cloudflared` by copying the host binary again.
- Revoke everything: delete the Access policy, or `camd.ctl revoke-all`, or stop `habitacion-cam-tunnel`.
- Remove it all: delete the DNS record `cam`, the tunnel and the Access app (account `cristobal.cc`), then stop the unit.
- From home the name resolves to the LAN address only on devices that use AdGuard (`192.168.1.2`); the router hands out Movistar's DNS,
  so phones normally go out through Cloudflare. Both paths work.

Notes: Movistar blocks Cloudflare on match days; at home use AdGuard as DNS to keep the direct HTTPS path. The VPS failover is
intentionally not used for this host: the video would pass through a third machine.

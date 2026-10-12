# Exposing the camera outside the house (not done yet)

Waiting for his explicit OK. Plan, in this order, verifying each step before the next:

1. **Access app first.** In the `cristobal.cc` Cloudflare account create a Zero Trust Access application for
   `cam.cristobal.cc` with a policy that allows only his email (one-time PIN). Do this before the DNS record
   exists, otherwise the hostname is briefly public without a login (see the lessons note for the Proxmox app).
2. **Tunnel ingress.** Add `cam.cristobal.cc -> http://192.168.1.73:8180` to the `cristobal.cc` account tunnel
   (`fec3231e-...`), or run a dedicated `cloudflared` inside LXC 131 pointing to `http://127.0.0.1:8180`.
3. **DNS.** The wildcard already resolves `*.cristobal.cc` to Cloudflare, so the explicit CNAME to the tunnel is what
   routes it. At home AdGuard keeps answering with the LAN address.
4. **Check.** From mobile data: Access login first, then the passkey login. Without Access the app must not load.
5. **Security pass.** `nikto`-style header check, confirm the camera port 80/81 is unreachable from outside,
   rotate the camera token if it was ever typed in a shared place.

Notes: Movistar blocks Cloudflare on match days; the LAN HTTPS path keeps working then. The failover to the VPS
is intentionally not used for this host: the video would pass through a third machine.

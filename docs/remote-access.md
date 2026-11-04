# HTTPS and remote access

camd listens on plain HTTP and expects a reverse proxy or tunnel to add HTTPS. The camera itself must never be exposed: only the server talks to it.

## Option 1: Cloudflare Tunnel (free, no open ports)

You need a domain on Cloudflare.

1. Zero Trust, Networks, Tunnels, create a tunnel and install `cloudflared` on the server with the token it shows.
2. Add a public hostname, for example `camera.example.org`, pointing to `http://127.0.0.1:8180`.
3. Set `origin = "https://camera.example.org"`, `rp_id = "camera.example.org"`, `secure_cookies = true`, `trust_proxy = true`.
4. Recommended: put a Cloudflare Access policy in front so two gates stand between the internet and your camera (Access, then the app's own login).

Cloudflare terminates TLS, so it can technically see the traffic. If that is not acceptable, use option 2 or 3.

## Option 2: a reverse proxy with a real certificate

Caddy gets and renews certificates by itself:

```
camera.example.org {
	@live path /api/live
	reverse_proxy @live 127.0.0.1:8180 {
		flush_interval -1
	}
	reverse_proxy 127.0.0.1:8180
}
```

For nginx see `deploy/nginx.conf` (get a certificate with certbot). To get a certificate for a name that only exists inside your LAN, use a DNS-01 challenge with `lego` or `certbot` and point the name to the server in your router's DNS.

## Option 3: a VPN

WireGuard or Tailscale to your home network and the plain address. Passkeys are not available over plain HTTP, but the password is.

## Settings that must match

| Setting | Value |
|---|---|
| `origin` | exactly what the browser shows, scheme and port included |
| `rp_id` | the host name only, no scheme, no port |
| `secure_cookies` | `true` over HTTPS |
| `trust_proxy` | `true` only when the proxy sets `X-Forwarded-For` (nginx, Caddy) or `CF-Connecting-IP` (Cloudflare) |

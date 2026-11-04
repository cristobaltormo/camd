# Security model

camd is meant to guard a private space, so the defaults are strict.

## What is protected and how

| Layer | Measure |
|---|---|
| Camera | Only the server talks to it. Every request needs a bearer token (compared in constant time). Over-the-air updates need the same token. The setup page only exists while the camera has no wifi configured or cannot reach it for two minutes. |
| Login | Passkeys (WebAuthn, user verification required) or a password hashed with scrypt. Ten attempts per minute per address. |
| Sessions | Random tokens stored hashed, 30 days, revocable one by one or all at once. Cookies are `HttpOnly`, `SameSite=Strict` and `Secure` behind HTTPS. |
| Requests | Writes must come from the same origin (CSRF). A strict content security policy forbids inline scripts and styles. |
| Files | Clips and photos are served only from inside the data directory. |
| Process | A dedicated user, a hardened systemd unit (`ProtectSystem=strict`, no new privileges, a syscall filter) or a non-root container. |

## What you should do

- Serve it over HTTPS if it is reachable from anywhere but your LAN.
- Use a long token. The setup page generates 24 hex characters.
- Keep the camera on a network you trust, or an IoT VLAN that can reach only the server.
- The setup access point is open and the wifi password crosses it unencrypted, once. Do the setup at home.
- Settings in the browser are applied by the logged-in owner only: there is one owner, no user roles.

## Privacy

Nothing is sent anywhere unless you configure a channel. The detection model runs locally. Web Push messages travel through your browser vendor's push service and contain only the title and text of the alert; ntfy and Notifich receive the same.

## Reporting a vulnerability

See [SECURITY.md](../SECURITY.md).

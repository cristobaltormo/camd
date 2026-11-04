# Security policy

If you find a vulnerability, please do not open a public issue. Use GitHub's private vulnerability reporting on this repository (Security, Report a vulnerability). You will get an answer within a week and a fix or a clear decision within 30 days.

Supported: the latest release. The threat model and what is already protected are in [docs/security.md](docs/security.md).

In scope: authentication and sessions, access to video and stored media, the camera protocol, the installer and the container image. Out of scope: problems that need an attacker with root on the server, and deployments that expose the app to the internet without HTTPS.

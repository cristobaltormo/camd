# Troubleshooting

Start with the log: `journalctl -u camd -n 100` or `docker compose logs --tail 100 camd`.

## The camera

**It never shows up on my wifi.** The ESP32 only joins 2.4 GHz networks. Make sure the router has 2.4 GHz enabled and that the network name has no unusual characters. If you mistyped the password the setup network `camd-setup-XXXX` comes back by itself after two minutes.

**The signal is very weak or it keeps disconnecting.** Fit the external antenna; with the antenna connector empty the level can be -80 dBm instead of -50. Also check the power supply: a charger that cannot deliver 1 A makes the board restart under load.

**I forgot the token.** Hold the serial console (`status` shows the state) and run `wipe`, or open the setup page from the serial console with `portal`. Then pair it again.

**Another viewer takes the stream.** The camera serves one stream client. Never open the camera's address in a browser while the server runs, and do not run two servers against the same camera.

**Stripes or a dark image.** Re-seat the ribbon cable of the sensor; stripes are almost always a loose cable. A dark image in a lit room usually means the sensor looks at a bright window.

## The server

**"Camera has no signal".** The server cannot reach the camera. Check `camera_host`, that the token matches, and that the camera answers: `curl -H "Authorization: Bearer TOKEN" http://CAMERA/status`. From Docker, `.local` names do not resolve: use the IP address and reserve it in your router.

**The app asks me to sign in again at every visit.** Cookies are `Secure` behind HTTPS and `SameSite=Strict`. Make sure `origin` is exactly the address you type, including the scheme and port, and that `secure_cookies` matches how you reach it.

**Passkeys are not offered.** They need HTTPS or `localhost`. Use a password on a plain HTTP LAN address, or see [remote access](remote-access.md).

**Writes fail with "bad origin".** The `Origin` header must match `origin` or the host you reached the server through. Behind a proxy set `trust_proxy = true` and pass `Host` and `X-Forwarded-For`.

**Detection never turns green.** Look in **Settings, Detection**. The first run downloads a 3.8 MB model; on a machine without internet copy `nanodet-plus-m.onnx` into `<data_dir>/models/` (see [install](install.md)). Until the detector is ready every motion is recorded and alerted, so nothing is lost.

**It does not see a person that is clearly there.** Lower **Minimum confidence** a little and add light. The detector needs to see the shape; a silhouette against a window is hard.

**Too many false alarms.** In `alert` mode only objects raise alerts; if you still get them, raise the confidence, remove objects you do not care about under **What to look for**, and draw ignored zones over fans, curtains or screens. Motion alone (mode `off`) is the noisiest setting.

**CPU is high.** Lower the resolution, and check that detection is not set to run on a busy scene all day: a TV that is always on counts as motion. Ignored zones fix that.

**Clips are not saved ("Could not be saved").** FFmpeg failed or the disk is full. The log has the reason. Free space or lower the retention.

**Auto mode never records when I am away.** Presence needs the IP address of your phone in `presence_devices`, with a fixed address reserved in the router. Phones sleep their wifi; there is a grace period (`presence_grace_s`) before the phone counts as away.

## The app

**An old version stays on my phone.** The app checks its build against the server and reloads itself. If it does not, close the app completely and open it again; a "There is a new version" message offers to reload.

**Notifications do not arrive on iPhone.** Web Push on iOS only works for an app added to the home screen, from iOS 16.4. Open the site in Safari, Share, Add to Home Screen, and enable alerts from the installed app. ntfy, which has its own iOS app, is an alternative.

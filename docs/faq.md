# FAQ

**Does it send anything to the cloud?** No. The video stays between the camera and your server. The detection model runs locally. Alerts go through the channel you configure (your browser vendor's push service, ntfy, or Notifich) and contain only a title and a line of text.

**Can I use more than one camera?** Yes. Add a `[[cameras]]` table per extra camera; see [several cameras](configuration.md#several-cameras). Each one keeps its own events and settings, and the app has a selector.

**Does it work with other ESP32-CAM boards?** The firmware has the AI-Thinker pin map. Other boards need their pins in `firmware/src/pins.h`; the server does not care which board it is, it only needs an MJPEG stream and a few control endpoints. See the [firmware API](firmware.md#http-api).

**Does it work with an IP camera?** Not out of the box. The server speaks the small HTTP protocol of the firmware. An MJPEG source that exposes `/capture` and `/stream` with a bearer token would work; an RTSP camera would need an adapter. Frigate and motionEye are better choices for those.

**Why not Frigate, motionEye or ESPHome?** They are excellent and more capable. camd exists for the case of one ESP32-CAM and one small machine: the firmware and the server are designed together, detection is built in and light, there is nothing to configure beyond a token, and it ships as an installable app. See the [comparison](../README.md#how-it-compares).

**Which objects can it recognise?** The 80 classes of COCO: people, cats, dogs, birds, cars, bicycles and so on. You choose which ones raise alerts.

**Why does it record when nothing was detected?** In `alert` mode (the default) every motion is recorded and only detections alert. Clips without objects are shown apart and deleted after a day. In `strict` mode they are never encoded.

**Is it safe to open it to the internet?** Put HTTPS in front of it and use passkeys, which are phishing resistant, or a long password. Better still, do not expose it: use a tunnel with an access policy, or a VPN. See [remote access](remote-access.md) and [security](security.md).

**Can I use it with Home Assistant?** Yes, with MQTT discovery. See [Home Assistant](home-assistant.md).

**What happens if my server restarts during an event?** The frames written so far are recovered and encoded when the service starts.

**How much disk does it use?** It depends on the scene. Set the retention in days and a size cap; the oldest unprotected clips go first. Protected events (the star) are never removed.

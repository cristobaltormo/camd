# Hardware

## The camera

| | |
|---|---|
| Board | AI-Thinker ESP32-CAM (ESP32-D0WD, 4 MB flash, 4 MB PSRAM) |
| Sensor | OV2640, 2 MP, no infrared filter removal |
| Programming | the ESP32-CAM-MB base board (micro-USB, enters flashing mode by itself) or a USB-serial adapter and a jumper |
| Wifi | 2.4 GHz only. Use the board with the external antenna connector and fit the antenna: without it the signal can be 30 dB weaker |
| Power | 5 V, at least 1 A. A weak USB charger or a thin cable is the most common cause of random restarts and `Brownout detector` messages |

Other ESP32-CAM boards (ESP32-S3, XIAO, TTGO) need their own pin map in `firmware/src/pins.h`.

## What to expect from the image

The OV2640 is a cheap sensor. At 800x600 and 10 to 15 frames per second it is more than enough to see who is in a room and for the detector to recognise a person, but it is soft in low light and blind in the dark. Plan for light: a lamp, a night light, or the flash LED through the `led` control. Modules sold as night vision (an OV2640 without the infrared cut filter, with infrared LEDs) use the same firmware and give a black and white image in the dark.

Resolution is set in **Settings, Camera**. 800x600 (SVGA) is the sweet spot; lower resolutions cut CPU and wifi use, higher ones rarely help detection because the network works on a 416x416 image.

## Placement

- Mount it where it sees the door and the floor: people are easier to recognise than a wall.
- Avoid pointing it at a window or a bright lamp; auto exposure will darken the rest of the room. Use ignored zones for anything that moves on its own, such as a fan or a screen.
- Keep it within 10 metres of the router with a clear path for the signal; check **Settings, System** for the wifi level (better than -70 dBm).
- A printed case protects the board and the lens; there are many for the ESP32-CAM-MB on Printables and Thingiverse.

## The server

Anything that runs Linux and stays on: a Raspberry Pi 4 or 5, a mini PC, a NAS with Docker, a Proxmox container. One camera uses about 120 to 150 MB of RAM and a few percent of a core (see [resources](../README.md#resources)), so the machine is more likely to be limited by disk than by CPU. Clip size depends on the scene; set the retention in days and the size cap in **Settings, Recording**.

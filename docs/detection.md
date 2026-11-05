# How detection works

Detection runs in two stages so that it stays cheap.

## 1. Motion

Every frame is decoded at a quarter of its size in grayscale, blurred and compared with a slowly adapting background. Pixels that differ more than a threshold are grouped, and the largest group decides whether there is motion. Two things protect against false alarms at this stage:

- a change of brightness that affects the whole image (the camera's auto exposure, a lamp turning on) is measured as the median difference and subtracted before comparing;
- if more than half of the image changes at once, the background is simply restarted.

Motion needs three consecutive frames before an event starts. Zones you draw in **Settings, Detection** are masked out.

## 2. Objects

While there is motion (and every few seconds while an event is open), a frame is sent to a small neural network. NanoDet-Plus-m comes from the [OpenCV Zoo](https://github.com/opencv/opencv_zoo), is 3.8 MB, and runs through OpenCV's own inference engine, so there is no extra runtime to install. The file is downloaded the first time it is needed and its SHA-256 is checked. It recognises the 80 COCO classes; you choose which ones count (default: person, cat, dog).

Only one frame is analysed at a time and frames that arrive meanwhile are skipped, so a slow machine falls behind gracefully instead of piling up work.

## What happens with the result

| Mode | Records | Alerts | Keeps |
|---|---|---|---|
| `off` | any motion | on every event | everything |
| `alert` (default) | any motion | only when an object is seen | clips without objects for `unconfirmed_keep_h` hours, shown apart as "No object" |
| `strict` | only events with an object | only when an object is seen | nothing without an object: they are dropped before being encoded |

If the model cannot be loaded (no internet on first start, for example) the server behaves as `off` until it is available, so nothing is missed.

A person who sits still keeps the event open: the network is asked again every 2.5 seconds while an event lasts, and an object found postpones the end of the clip.

## Tuning

- Too many events with no object: raise the motion sensitivity threshold (lower sensitivity) or draw ignored zones over windows and screens.
- Missed objects: lower the minimum confidence (try 40%), raise the resolution of the camera, or make sure the room is lit: the OV2640 sensor has no infrared.
- Objects you do not care about: untick them in **What to look for**.
- Alerts only for one part of the room: draw **Alert zones**. An object raises an alert only when the centre of its box is inside one of them; with none drawn it alerts everywhere. Ignored zones still apply to motion.

The thumbnail of an event is the frame where the object was seen most clearly, and the boxes are stored with the event so the player can show them.

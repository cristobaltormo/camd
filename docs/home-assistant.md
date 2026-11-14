# Home Assistant and MQTT

Set `mqtt_host` (and `mqtt_port` if it is not 1883) and camd publishes its state and, with `mqtt_discovery = true`, announces itself to Home Assistant. No YAML needed.

| Entity | Type | Topic (default prefix `camd`) |
|---|---|---|
| Motion | binary sensor | `camd/motion` |
| Object detected | binary sensor | `camd/object` |
| Camera | connectivity | `camd/camera` |
| Armed | binary sensor | `camd/armed` |
| Mode | select | `camd/mode`, set with `camd/mode/set` (`auto`, `armed`, `disarmed`) |
| Privacy | switch | `camd/privacy`, set with `camd/privacy/set` (`ON`, `OFF`) |

State topics are retained and the availability topic is `camd/status`.

## Presence from Home Assistant

Instead of pinging phones you can tell camd who is home: publish `home` or `away` to `camd/presence/set` (for example from an automation using your person entity). Publish `auto` to go back to pinging `presence_devices`.

## Several cameras

With `[[cameras]]` tables (see [several cameras](configuration.md#several-cameras)) each extra camera publishes under its own prefix, `<mqtt_topic>_<id>` unless its table sets `mqtt_topic`, and appears in Home Assistant as a device of its own with the same entities.

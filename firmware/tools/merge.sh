#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

BUILD=.pio/build/esp32cam
BOOT_APP0="$(find "${PLATFORMIO_CORE_DIR:-$HOME/.platformio}/packages/framework-arduinoespressif32/tools/partitions" -name boot_app0.bin | head -1)"

esptool --chip esp32 merge-bin -o camd-esp32cam.bin \
  --flash-mode dio --flash-freq 40m --flash-size 4MB \
  0x1000 "$BUILD/bootloader.bin" \
  0x8000 "$BUILD/partitions.bin" \
  0xe000 "$BOOT_APP0" \
  0x10000 "$BUILD/firmware.bin"

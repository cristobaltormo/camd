#!/usr/bin/env python3
"""Send serial commands to the camera and print its replies. Runs on the Mac."""
import sys
import time

import serial

PORT = "/dev/cu.usbserial-1120"
WAIT = float(sys.argv[1])
commands = sys.argv[2:] if sys.argv[2:] != ["-"] else sys.stdin.read().splitlines()

with serial.Serial(PORT, 115200, timeout=0.2) as ser:
    ser.dtr = False
    ser.rts = False
    time.sleep(0.3)
    for cmd in commands:
        ser.write((cmd + "\n").encode())
        end = time.time() + WAIT
        while time.time() < end:
            data = ser.read(512)
            if data:
                sys.stdout.write(data.decode(errors="replace"))
        sys.stdout.flush()

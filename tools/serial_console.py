import argparse
import sys
import time

import serial

BAUD = 115200
RESET_PAUSE_S = 0.3


def main() -> None:
    parser = argparse.ArgumentParser(description="Send commands to the camera over USB serial and print its replies")
    parser.add_argument("port", help="serial port, for example /dev/ttyUSB0 or /dev/cu.usbserial-1120")
    parser.add_argument("commands", nargs="+", help="commands such as 'ssid MyWifi', 'pass secret', 'token abc', 'status', 'reboot'; '-' reads them from stdin")
    parser.add_argument("--wait", type=float, default=2.0, help="seconds to listen after each command")
    args = parser.parse_args()
    commands = sys.stdin.read().splitlines() if args.commands == ["-"] else args.commands

    with serial.Serial(args.port, BAUD, timeout=0.2) as link:
        link.dtr = False
        link.rts = False
        time.sleep(RESET_PAUSE_S)
        for command in commands:
            link.write((command + "\n").encode())
            deadline = time.time() + args.wait
            while time.time() < deadline:
                data = link.read(512)
                if data:
                    sys.stdout.write(data.decode(errors="replace"))
            sys.stdout.flush()


if __name__ == "__main__":
    main()

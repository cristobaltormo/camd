import argparse
import getpass
import sys
import time

from . import __version__
from .auth import Auth, AuthError
from .config import load
from .db import Database


def show_devices(auth: Auth) -> None:
    for credential in auth.credentials():
        print(f"passkey  {credential['name']:<20} {time.strftime('%Y-%m-%d %H:%M', time.localtime(credential['created']))}")
    if auth.has_password():
        print("password set")
    for session in auth.sessions():
        print(f"session  {session['id']:<4} {session['user_agent'][:60]}")


def set_password(auth: Auth) -> None:
    password = getpass.getpass("New password: ")
    if password != getpass.getpass("Repeat it: "):
        raise AuthError("the two passwords differ")
    auth.set_password(password)
    print("Password saved")


def main() -> int:
    parser = argparse.ArgumentParser(prog="camd.ctl", description="Local administration of the camera service")
    parser.add_argument("--version", action="version", version=f"camd {__version__}")
    parser.add_argument("command", choices=("enroll", "password", "revoke-all", "devices"))
    command = parser.parse_args().command

    cfg = load()
    auth = Auth(cfg, Database(cfg.db_path))
    try:
        if command == "enroll":
            print(f"Enrollment code (valid 10 minutes): {auth.new_enroll_code()}")
        elif command == "password":
            set_password(auth)
        elif command == "revoke-all":
            auth.revoke_all()
            print("All sessions revoked")
        else:
            show_devices(auth)
    except AuthError as exc:
        print(exc, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

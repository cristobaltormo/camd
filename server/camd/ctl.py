"""Local administration: python -m camd.ctl enroll|revoke-all|devices"""
import sys
import time

from .auth import Auth
from .config import load
from .db import Database


def main() -> int:
    cfg = load()
    db = Database(cfg.db_path)
    auth = Auth(cfg, db)
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""

    if cmd == "enroll":
        print(f"Enrollment code (valid 10 minutes): {auth.new_enroll_code()}")
    elif cmd == "revoke-all":
        auth.revoke_all()
        print("All sessions revoked")
    elif cmd == "devices":
        for c in auth.credentials():
            print(f"passkey  {c['name']:<20} {time.strftime('%Y-%m-%d %H:%M', time.localtime(c['created']))}")
        for s in auth.sessions():
            print(f"session  {s['id']:<4} {s['user_agent'][:60]}")
    else:
        print(__doc__)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

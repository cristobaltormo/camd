"""Stamp every web asset URL with a content hash so a release can never mix old and new files.

Run by deploy.sh before shipping. Idempotent: the hash is computed over the files with their stamps stripped.
"""
import hashlib
import re
from pathlib import Path

WEB = Path(__file__).parent / "camd" / "web"
STAMPED = [WEB / "index.html", WEB / "sw.js", WEB / "js" / "main.js", *sorted((WEB / "js").glob("*.js"))]
ASSET = re.compile(r"((?:\./[a-z]+\.js)|(?:/style\.css)|(?:/theme\.js)|(?:/js/main\.js))(\?v=[0-9a-z.]+)?(?=[\"'])")
CACHE = re.compile(r'const CACHE = "cam-shell-[0-9a-z.]+";')
BUILD = re.compile(r'export const BUILD = "[0-9a-z.]+";')


def strip(text: str) -> str:
    text = ASSET.sub(lambda m: m.group(1), text)
    text = CACHE.sub('const CACHE = "cam-shell-X";', text)
    return BUILD.sub('export const BUILD = "X";', text)


def main() -> str:
    files = sorted({*STAMPED, *(p for p in WEB.rglob("*") if p.is_file() and p.suffix in {".css", ".webmanifest", ".svg", ".png"})})
    digest = hashlib.sha1()
    for path in files:
        raw = path.read_bytes()
        digest.update(path.name.encode())
        digest.update(strip(raw.decode()).encode() if path in STAMPED or path.suffix == ".css" else raw)
    build = digest.hexdigest()[:10]
    for path in STAMPED:
        text = path.read_text()
        text = ASSET.sub(lambda m: f"{m.group(1)}?v={build}", text)
        text = CACHE.sub(f'const CACHE = "cam-shell-{build}";', text)
        text = BUILD.sub(f'export const BUILD = "{build}";', text)
        path.write_text(text)
    (WEB.parent / "build.txt").write_text(build + "\n")
    return build


if __name__ == "__main__":
    print(main())

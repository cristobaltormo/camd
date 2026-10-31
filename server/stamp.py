import hashlib
import re
import sys
from pathlib import Path

ASSET = re.compile(r"((?:\./[a-z]+\.js)|(?:/style\.css)|(?:/theme\.js)|(?:/js/main\.js))(\?v=[0-9a-z.]+)?(?=[\"'])")
CACHE = re.compile(r'const CACHE = "cam-shell-[0-9a-z.]+";')
BUILD = re.compile(r'export const BUILD = "[0-9a-z.]+";')
BINARY_SUFFIXES = {".css", ".webmanifest", ".svg", ".png"}


def strip(text: str) -> str:
    text = ASSET.sub(lambda m: m.group(1), text)
    text = CACHE.sub('const CACHE = "cam-shell-X";', text)
    return BUILD.sub('export const BUILD = "X";', text)


def stamp(web: Path) -> str:
    stamped = [web / "index.html", web / "sw.js", *sorted((web / "js").glob("*.js"))]
    files = sorted({*stamped, *(p for p in web.rglob("*") if p.is_file() and p.suffix in BINARY_SUFFIXES)})
    digest = hashlib.sha1()
    for path in files:
        raw = path.read_bytes()
        digest.update(path.name.encode())
        digest.update(strip(raw.decode()).encode() if path in stamped or path.suffix == ".css" else raw)
    build = digest.hexdigest()[:10]
    for path in stamped:
        text = ASSET.sub(lambda m: f"{m.group(1)}?v={build}", path.read_text())
        text = CACHE.sub(f'const CACHE = "cam-shell-{build}";', text)
        text = BUILD.sub(f'export const BUILD = "{build}";', text)
        path.write_text(text)
    (web.parent / "build.txt").write_text(build + "\n")
    return build


if __name__ == "__main__":
    target = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).parent / "camd" / "web"
    print(stamp(target))

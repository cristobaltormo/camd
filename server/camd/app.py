import html
import logging
from contextlib import asynccontextmanager
from pathlib import Path
from urllib.parse import urlsplit

from fastapi import FastAPI, Request
from fastapi.responses import FileResponse, HTMLResponse, JSONResponse
from fastapi.staticfiles import StaticFiles

from .api import auth, control, events, live, notifications, photos
from .api.deps import Services
from .auth import Auth
from .config import Config
from .db import Database
from .runtime import Runtime
from .viewers import ViewerPool

log = logging.getLogger("camd.app")

WEB_DIR = Path(__file__).parent / "web"
BUILD_FILE = Path(__file__).parent / "build.txt"
MAX_VIEWERS = 4
SAFE_METHODS = ("GET", "HEAD", "OPTIONS")

CSP = (
    "default-src 'self'; img-src 'self' data: blob:; media-src 'self'; style-src 'self'; "
    "script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"
)
SECURITY_HEADERS = {
    "Content-Security-Policy": CSP,
    "X-Content-Type-Options": "nosniff",
    "Referrer-Policy": "no-referrer",
    "X-Frame-Options": "DENY",
}


def same_origin(request: Request, cfg: Config) -> bool:
    origin = request.headers.get("origin")
    if origin is None:
        return False
    return origin == cfg.origin or urlsplit(origin).netloc == request.headers.get("host")


def web_manifest(cfg: Config) -> dict:
    icons = [
        {"src": "/icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any"},
        {"src": "/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any"},
        {"src": "/icon-maskable-192.png", "sizes": "192x192", "type": "image/png", "purpose": "maskable"},
        {"src": "/icon-maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable"},
    ]
    return {
        "id": "/",
        "name": cfg.site_name,
        "short_name": cfg.site_name[:12],
        "description": f"{cfg.site_name}: live view, events and alerts",
        "lang": cfg.language,
        "dir": "ltr",
        "start_url": "/#/live",
        "scope": "/",
        "display": "standalone",
        "orientation": "any",
        "background_color": "#14161b",
        "theme_color": "#14161b",
        "categories": ["utilities", "lifestyle"],
        "icons": icons,
        "shortcuts": [
            {"name": "Live", "url": "/#/live", "icons": icons[:1]},
            {"name": "Events", "url": "/#/events", "icons": icons[:1]},
        ],
    }


def create_app(cfg: Config, db: Database | None = None, runtime: Runtime | None = None, start: bool = True) -> FastAPI:
    db = db or Database(cfg.db_path)
    runtime = runtime or Runtime(cfg, db)
    build = BUILD_FILE.read_text().strip() if BUILD_FILE.exists() else "dev"

    @asynccontextmanager
    async def lifespan(_app):
        if start:
            runtime.start()
        yield
        if start:
            runtime.stop()

    app = FastAPI(lifespan=lifespan, docs_url=None, redoc_url=None, openapi_url=None)
    app.state.services = Services(cfg, db, runtime, Auth(cfg, db), ViewerPool(MAX_VIEWERS), build)

    @app.middleware("http")
    async def guard(request: Request, call_next):
        if request.method not in SAFE_METHODS and not same_origin(request, cfg):
            return JSONResponse({"detail": "bad origin"}, status_code=403)
        response = await call_next(request)
        response.headers.update(SECURITY_HEADERS)
        if "cache-control" not in response.headers:
            response.headers["Cache-Control"] = "no-store" if request.url.path.startswith("/api/") else "private, no-cache"
        return response

    @app.get("/healthz")
    def healthz():
        return {"ok": True}

    @app.get("/")
    @app.get("/index.html")
    def index():
        page = (WEB_DIR / "index.html").read_text()
        return HTMLResponse(page.replace("{{site_name}}", html.escape(cfg.site_name)).replace("{{lang}}", cfg.language))

    @app.get("/manifest.webmanifest")
    def manifest():
        return JSONResponse(web_manifest(cfg), media_type="application/manifest+json")

    @app.get("/sw.js")
    def service_worker():
        return FileResponse(WEB_DIR / "sw.js", media_type="text/javascript", headers={"Cache-Control": "no-cache"})

    for module in (auth, control, events, live, photos, notifications):
        app.include_router(module.router)
    app.mount("/", StaticFiles(directory=WEB_DIR, html=True), name="web")
    return app

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { launch, sleep } from "./cdp.mjs";

const BASE = process.env.BASE || "http://127.0.0.1:18499";
const OUT = process.env.OUT || "docs/images";
const COOKIE = process.env.COOKIE;
mkdirSync(OUT, { recursive: true });
const { send, js, close } = await launch({ port: 9341 });
await send("Network.enable");
if (COOKIE) await send("Network.setCookie", { name: "cam_session", value: COOKIE, url: BASE, httpOnly: true });
const get = (path) => fetch(BASE + path, { headers: COOKIE ? { cookie: `cam_session=${COOKIE}` } : {} }).then((r) => r.json());

const events = (await get("/api/events?limit=200&hide_unconfirmed=true")).events.filter((e) => e.status === "done");
const featured = events.find((e) => e.labels.includes("person")) || events[0];

const DESKTOP = { width: 1440, height: 900, mobile: false, scale: 1 };
const PHONE = { width: 390, height: 844, mobile: true, scale: 2 };
const boxes = "document.querySelectorAll('.viewer .box').length > 0";
const SHOTS = [
  ["live", "live", DESKTOP, "dark", null, boxes],
  ["events", "events", DESKTOP, "dark"],
  ["event", `events/${featured ? featured.id : ""}`, DESKTOP, "dark", null, "document.querySelector('.sheet video, .sheet .player')"],
  ["photos", "photos", DESKTOP, "dark"],
  ["timeline", "events", DESKTOP, "dark", "day"],
  ["detection", "settings/detection", DESKTOP, "dark"],
  ["live-light", "live", DESKTOP, "light", null, boxes],
  ["live-mobile", "live", PHONE, "dark", null, boxes],
  ["events-mobile", "events", PHONE, "dark"],
  ["detection-mobile", "settings/detection", PHONE, "dark"],
];

const waitFor = async (expression, ms) => {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try { if (await js(expression)) return true; } catch (_) {}
    await sleep(100);
  }
  return false;
};

const view = (process.env.VIEW || "").split(",").filter(Boolean);
const themes = (process.env.THEMES || "dark").split(",");
const routes = (process.env.ROUTES || "").split(",").filter(Boolean);
const extra = routes.flatMap((route) => (view.length ? view : ["desktop"]).flatMap((size) => themes.map((theme) =>
  [`${size}-${theme}-${route.replace(/\//g, "-")}`, route, size === "phone" ? PHONE : DESKTOP, theme, process.env.EVENTS_VIEW || null])));

for (const [name, route, metrics, theme, mode, ready] of extra.length ? extra : SHOTS) {
  await send("Emulation.setDeviceMetricsOverride", { width: metrics.width, height: metrics.height, deviceScaleFactor: metrics.scale, mobile: metrics.mobile });
  await send("Page.navigate", { url: BASE + "/" });
  await sleep(600);
  await js(`localStorage.setItem('cam.theme', '${theme}'); localStorage.setItem('cam.language', 'en'); localStorage.setItem('cam.eventsView', JSON.stringify('${mode || "list"}')); location.hash = '#/${route}'; location.reload()`);
  await sleep(2500);
  if (ready && !(await waitFor(ready, 40000))) console.log("not ready, captured anyway:", name);
  await sleep(ready === boxes ? 0 : 300);
  const data = (await send("Page.captureScreenshot", { format: "png" })).result.data;
  writeFileSync(join(OUT, `${name}.png`), Buffer.from(data, "base64"));
  console.log("saved", name);
}
await close();
process.exit(0);

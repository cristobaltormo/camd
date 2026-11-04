import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { launch, sleep } from "./cdp.mjs";

const BASE = process.env.BASE || "http://127.0.0.1:8199";
const OUT = process.env.OUT || "docs/images";
mkdirSync(OUT, { recursive: true });
const { send, js, close } = await launch({ port: 9341 });

const DESKTOP = { width: 1440, height: 900, mobile: false, scale: 1 };
const PHONE = { width: 390, height: 844, mobile: true, scale: 2 };
const SHOTS = [
  ["live", "live", DESKTOP, "dark"],
  ["events", "events", DESKTOP, "dark"],
  ["event", "events/1", DESKTOP, "dark"],
  ["photos", "photos", DESKTOP, "dark"],
  ["detection", "settings/detection", DESKTOP, "dark"],
  ["live-light", "live", DESKTOP, "light"],
  ["live-mobile", "live", PHONE, "dark"],
  ["events-mobile", "events", PHONE, "dark"],
  ["detection-mobile", "settings/detection", PHONE, "dark"],
];

const extra = (process.env.ROUTES || "").split(",").filter(Boolean).map((route) => [`review-${route.replace("/", "-")}`, route, process.env.PHONE ? PHONE : DESKTOP, "dark"]);
for (const [name, route, view, theme] of extra.length ? extra : SHOTS) {
  await send("Emulation.setDeviceMetricsOverride", { width: view.width, height: view.height, deviceScaleFactor: view.scale, mobile: view.mobile });
  await send("Page.navigate", { url: BASE + "/" });
  await sleep(600);
  await js(`localStorage.setItem('cam.theme', '${theme}'); localStorage.setItem('cam.language', 'en'); location.hash = '#/${route}'; location.reload()`);
  await sleep(2500);
  const data = (await send("Page.captureScreenshot", { format: "png" })).result.data;
  writeFileSync(join(OUT, `${name}.png`), Buffer.from(data, "base64"));
  console.log("saved", name);
}
await close();
process.exit(0);

import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BASE = process.env.BASE || "http://127.0.0.1:8199";
const OUT = process.env.OUT || "docs/images";
const PORT = 9341;
const profile = mkdtempSync(join(tmpdir(), "camd-shots-"));
mkdirSync(OUT, { recursive: true });
const chrome = spawn(process.env.CHROMIUM || "chromium", ["--headless=new", "--no-sandbox", "--disable-gpu", "--hide-scrollbars", `--remote-debugging-port=${PORT}`, `--user-data-dir=${profile}`, "about:blank"], { stdio: "ignore" });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let wsUrl;
for (let i = 0; i < 60 && !wsUrl; i++) {
  try { wsUrl = (await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json()).find((t) => t.type === "page")?.webSocketDebuggerUrl; } catch (_) {}
  if (!wsUrl) await sleep(250);
}
const ws = new WebSocket(wsUrl);
await new Promise((r) => ws.addEventListener("open", r));
let nextId = 0;
const waiting = new Map();
ws.addEventListener("message", (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && waiting.has(msg.id)) { waiting.get(msg.id)(msg); waiting.delete(msg.id); }
});
const send = (method, params = {}) => new Promise((resolve) => { const id = ++nextId; waiting.set(id, resolve); ws.send(JSON.stringify({ id, method, params })); });
const js = async (expression) => (await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true })).result.result?.value;
await send("Page.enable");
await send("Runtime.enable");

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
ws.close();
chrome.kill();
try { rmSync(profile, { recursive: true, force: true, maxRetries: 5 }); } catch (_) {}
process.exit(0);

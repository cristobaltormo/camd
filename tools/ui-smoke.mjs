// Opens every route of a running instance in headless Chromium and fails on page errors or missing icons.
// BASE=https://cam.cristobal.cc COOKIE=<session token> RESOLVE="MAP cam.cristobal.cc 192.168.1.73" SHOTS=/tmp/dir node --experimental-websocket tools/ui-smoke.mjs
import { spawn } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BASE = process.env.BASE;
const COOKIE = process.env.COOKIE;
const SHOTS = process.env.SHOTS;
if (!BASE || !COOKIE) throw new Error("BASE and COOKIE are required");
if (SHOTS) mkdirSync(SHOTS, { recursive: true });
const PORT = 9334;
const profile = mkdtempSync(join(tmpdir(), "ui-smoke-"));
const args = ["--headless=new", "--no-sandbox", "--disable-gpu", `--remote-debugging-port=${PORT}`, `--user-data-dir=${profile}`, "--window-size=390,844", "about:blank"];
if (process.env.RESOLVE) args.unshift(`--host-resolver-rules=${process.env.RESOLVE}`);
const chrome = spawn("chromium", args, { stdio: "ignore" });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const problems = [];

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
  if (msg.id && waiting.has(msg.id)) { waiting.get(msg.id)(msg); waiting.delete(msg.id); return; }
  if (msg.method === "Runtime.exceptionThrown") problems.push("exception: " + (msg.params.exceptionDetails.exception?.description || msg.params.exceptionDetails.text));
  if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") problems.push("console.error: " + msg.params.args.map((a) => a.value ?? a.description).join(" "));
});
const send = (method, params = {}) => new Promise((resolve) => { const id = ++nextId; waiting.set(id, resolve); ws.send(JSON.stringify({ id, method, params })); });
const js = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (r.result.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description || "evaluate failed");
  return r.result.result.value;
};
await send("Runtime.enable");
await send("Page.enable");
await send("Network.enable");
await send("Network.setCookie", { name: "cam_session", value: COOKIE, url: BASE, secure: true, httpOnly: true });

let failed = 0;
const check = async (name, fn) => { try { await fn(); console.log("PASS", name); } catch (e) { failed++; console.log("FAIL", name, "-", e.message); } };
const waitFor = async (expr, label, ms = 15000) => {
  const end = Date.now() + ms;
  while (Date.now() < end) { try { if (await js(expr)) return; } catch (_) {} await sleep(150); }
  throw new Error("timeout: " + label);
};
const shot = async (name) => {
  if (!SHOTS) return;
  const r = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(SHOTS, name + ".png"), Buffer.from(r.result.data, "base64"));
};

await send("Page.navigate", { url: BASE + "/#/live" });
await sleep(1500);
await check("shell with four icons", async () => {
  await waitFor("document.querySelectorAll('.tabs .tab').length === 4", "tabs");
  const missing = await js("[...document.querySelectorAll('.tabs .tab')].filter((t) => !t.querySelector('svg')).map((t) => t.textContent)");
  if (missing.length) throw new Error("tabs without icon: " + missing);
});
await check("live stream shows frames", async () => { await waitFor("document.querySelector('.badge.live') && !document.querySelector('.badge.live').hidden", "live badge", 25000); await shot("live"); });
for (const route of ["events", "photos", "settings/alerts", "settings/camera", "settings/captures", "settings/system"]) {
  await check("route " + route, async () => {
    await js(`location.hash = '#/${route}'`);
    await waitFor(`document.querySelector('.page-head') && !document.querySelector('.loading') && !document.querySelector('.empty h3')?.textContent.includes('No se pudo')`, "view", 15000);
    await sleep(500);
    await shot(route.replace("/", "-"));
  });
}
await check("build matches the server", async () => {
  const [client, server] = await js("fetch('/api/status').then((r) => r.json()).then((s) => [new URL(document.querySelector('script[type=module]').src).searchParams.get('v'), s.build])");
  if (client !== server) throw new Error(`${client} vs ${server}`);
});
if (problems.length) { failed++; console.log("FAIL page errors:\n  " + [...new Set(problems)].join("\n  ")); } else console.log("PASS no page errors");

ws.close();
chrome.kill();
rmSync(profile, { recursive: true, force: true });
process.exit(failed ? 1 : 0);

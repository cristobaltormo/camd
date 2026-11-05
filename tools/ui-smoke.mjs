import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { launch, sleep } from "./cdp.mjs";

const BASE = process.env.BASE;
const COOKIE = process.env.COOKIE;
const SHOTS = process.env.SHOTS;
if (!BASE || !COOKIE) throw new Error("BASE and COOKIE are required");
if (SHOTS) mkdirSync(SHOTS, { recursive: true });
const problems = [];

const browser = await launch({ port: 9334, width: 390, height: 844, extraArgs: process.env.RESOLVE ? [`--host-resolver-rules=${process.env.RESOLVE}`] : [] });
const { send, js } = browser;
browser.onEvent((msg) => {
  if (msg.method === "Runtime.exceptionThrown") problems.push("exception: " + (msg.params.exceptionDetails.exception?.description || msg.params.exceptionDetails.text));
  if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") problems.push("console.error: " + msg.params.args.map((a) => a.value ?? a.description).join(" "));
});
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

await browser.close();
process.exit(failed ? 1 : 0);

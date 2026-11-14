const BASE = process.env.BASE || "http://127.0.0.1:18499";
const PASSWORD = process.env.PASSWORD;
const MOTION_EVENTS = Number(process.env.MOTION_EVENTS || 2);
const CLIPS = Number(process.env.CLIPS || 2);
const PHOTOS = Number(process.env.PHOTOS || 3);
const FINAL_MODE = process.env.FINAL_MODE || "armed";
const DEADLINE_MS = Number(process.env.SEED_TIMEOUT_MS || 180000);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const started = Date.now();
const log = (text) => process.stderr.write(`seed ${Math.round((Date.now() - started) / 1000)}s: ${text}\n`);
let cookie = "";

async function call(path, body, method = body === undefined ? "GET" : "POST") {
  const res = await fetch(BASE + path, {
    method,
    headers: { origin: BASE, cookie, ...(body === undefined ? {} : { "content-type": "application/json" }) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${await res.text()}`);
  const setCookie = res.headers.get("set-cookie");
  if (setCookie) cookie = setCookie.split(";")[0];
  return res.status === 204 ? null : res.json();
}

async function until(label, check, ms = DEADLINE_MS - (Date.now() - started)) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const value = await check().catch(() => null);
    if (value) return value;
    await sleep(500);
  }
  throw new Error("timeout waiting for " + label);
}

const events = async () => (await call("/api/events?limit=200")).events;
const doneMotion = async () => (await events()).filter((e) => e.kind === "motion" && e.status === "done");
const settled = async () => !(await events()).some((e) => e.status === "recording");

if (!PASSWORD) throw new Error("PASSWORD is required");
for (let attempt = 0; ; attempt++) {
  try { await call("/api/auth/login/password", { password: PASSWORD }); break; } catch (e) {
    if (attempt > 40) throw e;
    await sleep(250);
  }
}
log("signed in");
await call("/api/settings", { post_roll_s: 3, pre_roll_s: 1, detection: "alert", detect_labels: ["person", "cat", "dog"], ignore_rects: [[0.04, 0.06, 0.2, 0.16]] });
await call("/api/mode", { mode: "armed" });
await until("the camera", async () => (await call("/api/status")).camera.online);
log("camera online");

for (let i = 0; i < PHOTOS; i++) await call("/api/photos", {});
log(`${PHOTOS} photos`);

if (MOTION_EVENTS) {
  await until(`${MOTION_EVENTS} motion events`, async () => (await doneMotion()).length >= MOTION_EVENTS);
  log("motion events recorded");
}
await call("/api/mode", { mode: "disarmed" });
await until("the last motion event to close", settled);

for (let i = 0; i < CLIPS; i++) {
  await call("/api/clips/start", { seconds: 5 });
  await sleep(3500);
  await call("/api/clips/stop", {}).catch(() => null);
  await until("the clip to be saved", settled);
}
log(`${CLIPS} clips`);
const clip = (await events()).find((e) => e.kind === "manual" && e.status === "done");
if (clip) await call(`/api/events/${clip.id}/keep`, { keep: true });

await call("/api/mode", { mode: FINAL_MODE });
const all = await events();
log(`done: ${all.length} events (${all.filter((e) => e.labels.includes("person")).length} with a person), mode ${FINAL_MODE}`);
process.stdout.write(cookie.replace(/^cam_session=/, "") + "\n");

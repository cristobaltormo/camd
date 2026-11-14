import { launch, sleep } from "./cdp.mjs";

const BASE = process.env.UI_BASE || "http://127.0.0.1:8199";
const problems = [];

const COOKIE = process.env.COOKIE;
const browser = await launch({ port: 9333 });
const { send, js } = browser;
await send("Network.enable");
if (COOKIE) await send("Network.setCookie", { name: "cam_session", value: COOKIE, url: BASE, httpOnly: true });
const request = (path, opts = {}) => fetch(BASE + path, { ...opts, headers: { origin: BASE, ...(COOKIE ? { cookie: `cam_session=${COOKIE}` } : {}), ...(opts.body ? { "content-type": "application/json" } : {}) } });
const get = (path) => request(path).then((r) => r.json());
const isNoise = (e) => e.analyzed && e.kind === "motion" && !e.labels.length && e.status === "done";
const shownEvents = async () => (await get("/api/events?limit=200&hide_unconfirmed=true")).events.filter((e) => !isNoise(e));
const doneEvents = async () => (await shownEvents()).filter((e) => e.status === "done");
browser.onEvent((msg) => {
  if (msg.method === "Runtime.exceptionThrown") problems.push("exception: " + (msg.params.exceptionDetails.exception?.description || msg.params.exceptionDetails.text));
  if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") problems.push("console.error: " + msg.params.args.map((a) => a.value ?? a.description).join(" "));
});

const waitFor = async (expression, label, ms = 8000) => {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try { if (await js(expression)) return; } catch (_) {}
    await sleep(100);
  }
  throw new Error("timeout waiting for: " + label);
};
const goto = async (hash) => { await send("Page.navigate", { url: `${BASE}/#${hash}` }); await sleep(300); await js("location.reload()").catch(() => {}); await sleep(500); };
const click = (selector) => js(`document.querySelector(${JSON.stringify(selector)}).click()`);
const clickText = (selector, text) => js(`[...document.querySelectorAll(${JSON.stringify(selector)})].find((b) => b.textContent.includes(${JSON.stringify(text)})).click()`);
const serverStatus = () => get("/api/status");
const streaming = "document.querySelector('.tag.live') && !document.querySelector('.tag.live').hidden";
const flipPrivacy = async () => {
  await waitFor("!document.querySelector('input[aria-label=\"Privacy\"]').disabled", "privacy switch ready");
  await click('input[aria-label="Privacy"]');
};
const count = (selector) => js(`document.querySelectorAll(${JSON.stringify(selector)}).length`);

const tests = {
  async "live view starts streaming"() {
    await goto("/live");
    await waitFor("document.querySelector('.tag.live') && !document.querySelector('.tag.live').hidden", "live badge");
  },
  async "privacy on shows the standby state and keeps the switch"() {
    await click('input[aria-label="Privacy"]');
    await waitFor("document.querySelector('.overlay')?.textContent.includes('off for privacy')", "standby overlay");
    await sleep(4600);
    if (!(await js("document.querySelector('input[aria-label=\"Privacy\"]').checked"))) throw new Error("switch reset by polling");
  },
  async "privacy off wakes the camera and resumes the stream without reloading"() {
    await click('input[aria-label="Privacy"]');
    await waitFor(`document.querySelector('.overlay')?.textContent.includes('Turning the camera on') || (${streaming})`, "waking overlay or stream", 4000);
    await waitFor(streaming, "stream back", 15000);
    if (await js("document.querySelector('input[aria-label=\"Privacy\"]').checked")) throw new Error("switch still on");
  },
  async "rapid privacy toggling ends consistent with the server"() {
    for (let i = 0; i < 3; i++) { await click('input[aria-label="Privacy"]'); await sleep(250); }
    await sleep(6000);
    const server = (await serverStatus()).privacy;
    const ui = await js("document.querySelector('input[aria-label=\"Privacy\"]').checked");
    if (server !== ui) throw new Error(`server ${server} vs ui ${ui}`);
    if (server) { await click('input[aria-label="Privacy"]'); await sleep(500); }
    await waitFor("document.querySelector('.tag.live') && !document.querySelector('.tag.live').hidden", "stream back", 15000);
  },
  async "privacy and camera overlays carry no stray text"() {
    await flipPrivacy();
    await waitFor("document.querySelector('.overlay')?.textContent.includes('off for privacy')", "standby overlay");
    const text = await js("document.querySelector('.overlay').innerText");
    if (/\b(null|undefined)\b/.test(text)) throw new Error("overlay says: " + text);
    await flipPrivacy();
    await waitFor(streaming, "stream back", 15000);
  },
  async "the sidebar and the live header agree on the camera state"() {
    await send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
    await goto("/live");
    await waitFor(streaming, "stream");
    await clickText(".seg button", "Disarmed");
    await waitFor("document.querySelector('.side-status').textContent === document.querySelector('.state-text').textContent && document.querySelector('.state-text').textContent === 'Not watching'", "both not watching", 4000);
    await flipPrivacy();
    await waitFor("document.querySelector('.side-status').textContent === 'Camera off' && document.querySelector('.state-text').textContent === 'Camera off'", "both camera off", 4000);
    await flipPrivacy();
    await waitFor(streaming, "stream back", 15000);
    await send("Emulation.clearDeviceMetricsOverride");
  },
  async "disconnecting the camera shows a calm state and reconnecting resumes"() {
    await goto("/live");
    await waitFor(streaming, "stream");
    await js("fetch('/api/settings', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ camera_enabled: false }) })");
    await waitFor("document.querySelector('.overlay')?.textContent.includes('Camera disabled')", "disabled overlay", 8000);
    if (!(await js("document.querySelector('input[aria-label=\"Privacy\"]').disabled && [...document.querySelectorAll('.btn.action')].every((b) => b.disabled)"))) throw new Error("controls still enabled");
    await clickText(".overlay .btn", "Enable camera");
    await waitFor(`document.querySelector('.overlay')?.textContent.includes('Looking for the camera') || (${streaming})`, "searching or stream", 5000);
    await waitFor("document.querySelector('.tag.live') && !document.querySelector('.tag.live').hidden", "stream back", 20000);
  },
  async "manual clip starts and stops"() {
    await clickText(".btn.action", "Clip");
    await waitFor("[...document.querySelectorAll('.btn.action')].some((b) => b.textContent.includes('Stop'))", "stop label");
    await clickText(".btn.action", "Stop");
    await waitFor("[...document.querySelectorAll('.btn.action')].some((b) => b.textContent.trim() === 'Clip')", "idle label");
  },
  async "photo asks where to save and stores on the web"() {
    await js("localStorage.removeItem('cam.photoDest')");
    const before = (await get("/api/photos")).photos.length;
    await clickText(".btn.action", "Photo");
    await waitFor("document.querySelectorAll('.choice').length === 2", "chooser");
    await click(".choice");
    await waitFor("[...document.querySelectorAll('.toast')].some((t) => t.textContent.includes('Photo saved'))", "toast");
    const after = (await get("/api/photos")).photos.length;
    if (after !== before + 1) throw new Error("photo not stored");
  },
  async "events show what was detected and keep the no-object noise apart"() {
    const shown = await shownEvents();
    const people = shown.filter((e) => e.labels.includes("person")).length;
    const noise = (await get("/api/events?limit=200")).events.filter(isNoise).length;
    if (shown.length < 3) throw new Error(`needs at least 3 events, the server has ${shown.length}`);
    await goto("/events");
    await waitFor(`document.querySelectorAll('.event').length === ${shown.length}`, `${shown.length} events without the noise`);
    if (people) {
      const tags = await js("[...document.querySelectorAll('.event .what-text')].map((t) => t.textContent.trim())");
      if (!tags.some((tag) => tag.startsWith("Person"))) throw new Error("person label missing: " + tags);
      await clickText(".filter", "People");
      await waitFor(`document.querySelectorAll('.event').length === ${people}`, `only the ${people} with people`);
    }
    if (noise) {
      await clickText(".filter", "No object");
      await waitFor(`document.querySelectorAll('.event').length === ${noise}`, "only the noise");
    } else if (await js("[...document.querySelectorAll('.filter')].some((b) => b.textContent === 'No object')")) throw new Error("noise tab without noise");
    await clickText(".filter", "All");
  },
  async "the day view draws a timeline and opens an event from it"() {
    await js("localStorage.setItem('cam.eventsView', JSON.stringify('day'))");
    await goto("/events");
    await waitFor("document.querySelectorAll('.timeline .mark').length >= 1", "marks on the timeline");
    const done = (await doneEvents()).map((e) => e.id);
    await js(`[...document.querySelectorAll('.timeline .mark')].find((m) => ${JSON.stringify(done)}.includes(Number(m.dataset.id))).click()`);
    await waitFor("document.querySelector('.sheet')", "player from the timeline");
    await goto("/events");
    await js("localStorage.removeItem('cam.eventsView')");
  },
  async "events list, delete from the player and protect"() {
    const before = (await shownEvents()).length;
    const [victim, other] = (await doneEvents()).filter((e) => !e.keep);
    if (!other) throw new Error("needs two unprotected events");
    await goto("/events");
    await waitFor(`document.querySelectorAll('.event').length === ${before}`, `${before} events`);
    await goto(`/events/${victim.id}`);
    await waitFor("document.querySelector('.sheet')", "player");
    await clickText(".sheet .btn", "Delete");
    await waitFor("document.querySelector('.center-modal')", "confirm");
    await click(".center-modal .danger-solid");
    await waitFor(`document.querySelectorAll('.event').length === ${before - 1}`, "list redrawn");
    const ids = (await get("/api/events?limit=200")).events.map((e) => e.id);
    if (ids.includes(victim.id)) throw new Error("event still on server");
    await waitFor("!document.querySelector('.sheet') || location.hash.startsWith('#/events/')", "navigated");
    await goto(`/events/${other.id}`);
    await waitFor("document.querySelector('.sheet')", "player " + other.id);
    await clickText(".sheet .btn", "Protect");
    await waitFor("[...document.querySelectorAll('.sheet .btn')].some((b) => b.textContent.includes('Protected'))", "protected label");
    if (!(await get("/api/events?limit=200")).events.find((e) => e.id === other.id).keep) throw new Error("not protected on the server");
  },
  async "bulk select and delete events"() {
    const before = (await shownEvents()).length;
    if (before < 3) throw new Error("needs at least 3 events");
    await goto("/events");
    await waitFor(`document.querySelectorAll('.event').length === ${before}`, "events");
    await clickText(".page-head .btn", "Select");
    await js("const done = [...document.querySelectorAll('.event')].filter((e) => !e.querySelector('.rec-dot')); done[0].click(); done[1].click()");
    await waitFor("document.querySelector('.bulkbar')?.textContent.includes('2 selected')", "two picked");
    await clickText(".bulkbar .btn", "Delete");
    await click(".center-modal .danger-solid");
    await waitFor(`document.querySelectorAll('.event').length === ${before - 2}`, `${before - 2} left`);
  },
  async "photos gallery shows and deletes"() {
    const photos = (await get("/api/photos")).photos;
    if (!photos.length) throw new Error("needs a photo");
    await goto("/photos");
    await waitFor(`document.querySelectorAll('.photo').length === ${photos.length}`, `${photos.length} photos`);
    await goto(`/photos/${photos[0].id}`);
    await waitFor("document.querySelector('.viewer-img')", "viewer");
    await clickText(".sheet .btn", "Delete");
    await click(".center-modal .danger-solid");
    await waitFor(`document.querySelectorAll('.photo').length === ${photos.length - 1}`, `${photos.length - 1} photos`);
  },
  async "detection settings offer the modes and the labels"() {
    await goto("/settings/detection");
    await waitFor("document.querySelectorAll('[aria-label=\"Detection mode\"] button').length === 3", "mode buttons");
    await waitFor("document.querySelectorAll('.checks .check').length >= 6", "label choices");
    await clickText('[aria-label="Detection mode"] button', "Only objects");
    await waitFor("!document.querySelector('.savebar').hidden", "save bar after changing the mode");
    await clickText(".savebar .btn", "Discard");
  },
  async "zones of both kinds share one editor"() {
    const { ignore_rects: ignored } = await get("/api/settings");
    if (!ignored.length) throw new Error("needs an ignored zone");
    await goto("/settings/detection");
    await waitFor("document.querySelector('.zone .rect.ignore.current')", "ignored zone drawn");
    await clickText('[aria-label="Zone type"] button', "Alert zones");
    await waitFor("!document.querySelector('.zone .rect.ignore.current') && document.querySelector('.zone-editor').textContent.includes('No zones')", "alert kind selected");
  },
  async "live lists what happened today"() {
    await goto("/live");
    await waitFor("document.querySelectorAll('.today .event').length >= 1 && document.querySelectorAll('.today .timeline .mark').length >= document.querySelectorAll('.today .event').length", "today rows and marks");
    const done = (await doneEvents()).map((e) => e.id);
    await js(`[...document.querySelectorAll('.today .event')].find((e) => ${JSON.stringify(done)}.includes(Number(e.dataset.id))).click()`);
    await waitFor("location.hash.startsWith('#/events/') && document.querySelector('.sheet')", "event opened from live");
  },
  async "settings on a phone start with the list of sections"() {
    await send("Emulation.setDeviceMetricsOverride", { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
    await goto("/settings");
    await waitFor("document.querySelectorAll('.settings-body .nav-row').length === 10", "section list");
    await click('.nav-row[href="#/settings/camera"]');
    await waitFor("document.querySelector('.page-head .back') && document.querySelector('.page-head h1').textContent === 'Camera'", "camera section with back link");
    await click(".page-head .back");
    await waitFor("document.querySelectorAll('.settings-body .nav-row').length === 10", "back to the list");
    await send("Emulation.clearDeviceMetricsOverride");
  },
  async "a toast never covers the save bar"() {
    await goto("/settings/alerts");
    await waitFor("document.querySelector('.settings-body .switch')", "settings");
    await click(".settings-body .switch");
    await waitFor("!document.querySelector('.savebar').hidden", "save bar");
    await js("import('/js/lib.js').then((lib) => lib.toast('Overlap probe', 'ok'))");
    await waitFor("[...document.querySelectorAll('.toast')].some((t) => t.textContent.includes('Overlap probe'))", "toast");
    const overlap = await js("(() => { const bar = document.querySelector('.savebar').getBoundingClientRect(); const toast = [...document.querySelectorAll('.toast')].pop().getBoundingClientRect(); return toast.bottom > bar.top && toast.top < bar.bottom; })()");
    if (overlap) throw new Error("toast overlaps the save bar");
    await clickText(".savebar .btn", "Discard");
  },
  async "settings shows the save bar only with changes"() {
    await goto("/settings/alerts");
    await waitFor("document.querySelector('.settings-body .switch')", "settings");
    if (!(await js("document.querySelector('.savebar').hidden"))) throw new Error("save bar visible without changes");
    await click(".settings-body .switch");
    await waitFor("!document.querySelector('.savebar').hidden", "save bar");
    await clickText(".savebar .btn", "Discard");
    await waitFor("document.querySelector('.savebar').hidden", "save bar hidden");
  },
  async "every section renders"() {
    for (const id of ["alerts", "detection", "recording", "captures", "presence", "camera", "appearance", "app", "system", "security"]) {
      await goto(`/settings/${id}`);
      await waitFor("document.querySelectorAll('.settings-body .group').length > 0", `section ${id}`);
    }
  },
};

let failed = 0;
for (const [name, fn] of Object.entries(tests)) {
  try { await fn(); console.log("PASS", name); } catch (e) { failed++; console.log("FAIL", name, "-", e.message); }
}
if (problems.length) { failed++; console.log("FAIL page errors:\n  " + [...new Set(problems)].join("\n  ")); } else console.log("PASS no page errors");

await browser.close();
process.exit(failed ? 1 : 0);

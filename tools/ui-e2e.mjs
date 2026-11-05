import { launch, sleep } from "./cdp.mjs";

const BASE = process.env.UI_BASE || "http://127.0.0.1:8199";
const problems = [];

const browser = await launch({ port: 9333 });
const { send, js } = browser;
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
const mockStatus = () => fetch(`${BASE}/api/status`).then((r) => r.json());
const count = (selector) => js(`document.querySelectorAll(${JSON.stringify(selector)}).length`);

const tests = {
  async "live view starts streaming"() {
    await goto("/live");
    await waitFor("document.querySelector('.badge.live') && !document.querySelector('.badge.live').hidden", "live badge");
  },
  async "privacy on shows the standby state and keeps the switch"() {
    await click('input[aria-label="Privacy"]');
    await waitFor("document.querySelector('.overlay')?.textContent.includes('off for privacy')", "standby overlay");
    await sleep(4600);
    if (!(await js("document.querySelector('input[aria-label=\"Privacy\"]').checked"))) throw new Error("switch reset by polling");
  },
  async "privacy off wakes the camera and resumes the stream without reloading"() {
    await click('input[aria-label="Privacy"]');
    await waitFor("document.querySelector('.overlay')?.textContent.includes('Turning the camera on')", "waking overlay", 4000);
    await waitFor("document.querySelector('.badge.live') && !document.querySelector('.badge.live').hidden", "stream back", 15000);
    if (await js("document.querySelector('input[aria-label=\"Privacy\"]').checked")) throw new Error("switch still on");
  },
  async "rapid privacy toggling ends consistent with the server"() {
    for (let i = 0; i < 3; i++) { await click('input[aria-label="Privacy"]'); await sleep(250); }
    await sleep(6000);
    const server = (await mockStatus()).privacy;
    const ui = await js("document.querySelector('input[aria-label=\"Privacy\"]').checked");
    if (server !== ui) throw new Error(`server ${server} vs ui ${ui}`);
    if (server) { await click('input[aria-label="Privacy"]'); await sleep(500); }
    await waitFor("document.querySelector('.badge.live') && !document.querySelector('.badge.live').hidden", "stream back", 15000);
  },
  async "disconnecting the camera shows a calm state and reconnecting resumes"() {
    await await js("fetch('/api/settings', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ camera_enabled: false }) })");
    await waitFor("document.querySelector('.overlay')?.textContent.includes('Camera disabled')", "disabled overlay", 8000);
    if (!(await js("document.querySelector('input[aria-label=\"Privacy\"]').disabled && [...document.querySelectorAll('.btn.action')].every((b) => b.disabled)"))) throw new Error("controls still enabled");
    await clickText(".overlay .btn", "Enable camera");
    await waitFor("document.querySelector('.overlay')?.textContent.includes('Looking for the camera')", "searching", 5000);
    await waitFor("document.querySelector('.badge.live') && !document.querySelector('.badge.live').hidden", "stream back", 20000);
  },
  async "manual clip starts and stops"() {
    await clickText(".btn.action", "Clip");
    await waitFor("[...document.querySelectorAll('.btn.action')].some((b) => b.textContent.includes('Stop'))", "stop label");
    await clickText(".btn.action", "Stop");
    await waitFor("[...document.querySelectorAll('.btn.action')].some((b) => b.textContent.trim() === 'Clip')", "idle label");
  },
  async "photo asks where to save and stores on the web"() {
    await js("localStorage.removeItem('cam.photoDest')");
    const before = (await fetch(`${BASE}/api/photos`).then((r) => r.json())).photos.length;
    await clickText(".btn.action", "Photo");
    await waitFor("document.querySelectorAll('.choice').length === 2", "chooser");
    await click(".choice");
    await waitFor("[...document.querySelectorAll('.toast')].some((t) => t.textContent.includes('Photo saved'))", "toast");
    const after = (await fetch(`${BASE}/api/photos`).then((r) => r.json())).photos.length;
    if (after !== before + 1) throw new Error("photo not stored");
  },
  async "events show what was detected and keep the no-object noise apart"() {
    await goto("/events");
    await waitFor("document.querySelectorAll('.event').length === 6", "6 events without the noise");
    const tags = await js("[...document.querySelectorAll('.event .tag')].map((t) => t.textContent.trim())");
    if (!tags.includes("Person") || !tags.includes("Cat")) throw new Error("labels missing: " + tags);
    await clickText(".filter", "People");
    await waitFor("document.querySelectorAll('.event').length === 1", "only people");
    await clickText(".filter", "No object");
    await waitFor("document.querySelectorAll('.event').length === 1", "only the noise");
    await clickText(".filter", "All");
  },
  async "the day view draws a timeline and opens an event from it"() {
    await js("localStorage.setItem('cam.eventsView', JSON.stringify('day'))");
    await goto("/events");
    await waitFor("document.querySelectorAll('.timeline .mark').length >= 1", "marks on the timeline");
    await click(".timeline .mark");
    await waitFor("document.querySelector('.sheet')", "player from the timeline");
    await goto("/events");
    await js("localStorage.removeItem('cam.eventsView')");
  },
  async "events list, delete from the player and protect"() {
    await goto("/events");
    await waitFor("document.querySelectorAll('.event').length === 6", "5 events");
    await goto("/events/3");
    await waitFor("document.querySelector('.sheet')", "player");
    await clickText(".sheet .btn", "Delete");
    await waitFor("document.querySelector('.center-modal')", "confirm");
    await click(".center-modal .danger-solid");
    await waitFor("document.querySelectorAll('.event').length === 5", "list redrawn");
    const ids = (await fetch(`${BASE}/api/events`).then((r) => r.json())).events.map((e) => e.id);
    if (ids.includes(3)) throw new Error("event still on server");
    await waitFor("!document.querySelector('.sheet') || location.hash.startsWith('#/events/')", "navigated");
    await goto("/events/1");
    await waitFor("document.querySelector('.sheet')", "player 1");
    await clickText(".sheet .btn", "Protect");
    await waitFor("[...document.querySelectorAll('.sheet .btn')].some((b) => b.textContent.includes('Protected'))", "protected label");
  },
  async "bulk select and delete events"() {
    await goto("/events");
    await waitFor("document.querySelectorAll('.event').length >= 5", "events");
    await clickText(".page-head .btn", "Select");
    await js("document.querySelectorAll('.event')[0].click(); document.querySelectorAll('.event')[1].click()");
    await waitFor("document.querySelector('.bulkbar')?.textContent.includes('2 selected')", "two picked");
    await clickText(".bulkbar .btn", "Delete");
    await click(".center-modal .danger-solid");
    await waitFor("document.querySelectorAll('.event').length === 3", "2 left");
  },
  async "photos gallery shows and deletes"() {
    await goto("/photos");
    await waitFor("document.querySelectorAll('.event.photo').length === 4", "4 photos");
    await goto("/photos/1");
    await waitFor("document.querySelector('.viewer-img')", "viewer");
    await clickText(".sheet .btn", "Delete");
    await click(".center-modal .danger-solid");
    await waitFor("document.querySelectorAll('.event.photo').length === 3", "3 photos");
  },
  async "detection settings offer the modes and the labels"() {
    await goto("/settings/detection");
    await waitFor("document.querySelectorAll('.seg button').length === 3", "mode buttons");
    await waitFor("document.querySelectorAll('.checks .check').length >= 6", "label choices");
    await clickText(".seg button", "Only objects");
    await waitFor("!document.querySelector('.savebar').hidden", "save bar after changing the mode");
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
      await waitFor("document.querySelectorAll('.settings-body .panel').length > 0", `section ${id}`);
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

import assert from "node:assert/strict";
import test from "node:test";

import { isNoise, matches, queryFor, reconcile } from "../camd/web/js/events.js";
import { deviceLabel, fmtAgo, fmtBytes, fmtDur, fmtUptime, labelName } from "../camd/web/js/lib.js";
import { count, resolveLanguage, setLanguage, t } from "../camd/web/js/text.js";

const now = () => Date.now() / 1000;
const event = (over = {}) => ({ id: 1, start: now(), kind: "motion", keep: false, status: "done", analyzed: false, labels: [], ...over });

test.beforeEach(() => setLanguage("en"));

test("durations", () => {
  assert.equal(fmtDur(null), "Recording");
  assert.equal(fmtDur(0.2), "1 s");
  assert.equal(fmtDur(45), "45 s");
  assert.equal(fmtDur(61), "1:01");
});

test("relative time", () => {
  assert.equal(fmtAgo(now()), "just now");
  assert.equal(fmtAgo(now() - 300), "5 min ago");
  assert.equal(fmtAgo(now() - 7300), "2 h ago");
  assert.equal(fmtAgo(now() - 3 * 86400), "3 d ago");
});

test("sizes and uptime", () => {
  assert.equal(fmtBytes(512), "1 KB");
  assert.equal(fmtBytes(5 * 1024 ** 2), "5 MB");
  assert.equal(fmtBytes(3 * 1024 ** 3), "3.0 GB");
  assert.equal(fmtUptime(0), "-");
  assert.equal(fmtUptime(90000), "1 d 1 h");
});

test("device labels", () => {
  assert.equal(deviceLabel("Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit Version/27 Mobile Safari/604.1"), "iPhone · Safari");
  assert.equal(deviceLabel("Mozilla/5.0 (Windows NT 10.0) Chrome/120 Safari/537"), "Windows · Chrome");
  assert.equal(deviceLabel(""), "Unknown device");
});

test("the language follows the preference, then the browser, then English", () => {
  assert.equal(resolveLanguage("es", "en-US"), "es");
  assert.equal(resolveLanguage("auto", "es-ES"), "es");
  assert.equal(resolveLanguage("auto", "fr-FR"), "en");
  assert.equal(resolveLanguage("xx", "es"), "es");
});

test("strings are translated with variables and plurals", () => {
  setLanguage("es");
  assert.equal(t("Today"), "Hoy");
  assert.equal(t("{n} min ago", { n: 5 }), "hace 5 min");
  assert.equal(count(1, "{n} event", "{n} events"), "1 evento");
  assert.equal(count(3, "{n} event", "{n} events"), "3 eventos");
  assert.equal(t("not in the catalog"), "not in the catalog");
});

test("object names are translated and unknown ones are capitalised", () => {
  assert.equal(labelName("person"), "Person");
  setLanguage("es");
  assert.equal(labelName("person"), "Persona");
  assert.equal(labelName("zebra"), "Cebra");
  assert.equal(labelName("flux capacitor"), "Flux capacitor");
});

test("event filters", () => {
  const ev = event();
  assert.ok(matches("all", ev) && matches("today", ev) && matches("week", ev));
  assert.ok(!matches("yesterday", ev) && !matches("clips", ev) && !matches("kept", ev));
  assert.ok(matches("clips", { ...ev, kind: "manual" }));
  assert.ok(matches("kept", { ...ev, keep: true }));
});

test("object filters and the no-object noise", () => {
  const person = event({ analyzed: true, labels: ["person"] });
  const cat = event({ analyzed: true, labels: ["cat", "person"] });
  const noise = event({ analyzed: true });
  assert.ok(matches("people", person) && !matches("animals", person));
  assert.ok(matches("animals", cat) && matches("people", cat));
  assert.ok(isNoise(noise) && !isNoise(person) && !isNoise(event()));
  assert.ok(!matches("all", noise) && !matches("today", noise) && matches("noise", noise) && !matches("noise", person));
  assert.ok(!isNoise(event({ analyzed: true, kind: "manual" })) && !isNoise(event({ analyzed: true, status: "recording" })));
});

test("reconcile drops deleted events inside the fetched range and keeps older pages", () => {
  const e = (id, start) => ({ id, start });
  const current = [e(5, 500), e(4, 400), e(3, 300), e(2, 200), e(1, 100)];
  const fresh = [e(6, 600), e(5, 500), e(3, 300)];
  assert.deepEqual(reconcile(current, fresh, 3).map((x) => x.id), [6, 5, 3, 2, 1]);
  assert.deepEqual(reconcile(current, [e(5, 500)], 60).map((x) => x.id), [5]);
  assert.deepEqual(reconcile(current, [], 60), []);
});

test("the list asks the server for what the filter needs", () => {
  assert.equal(queryFor("all"), "&hide_unconfirmed=true");
  assert.equal(queryFor("people"), "&hide_unconfirmed=true&label=person");
  assert.equal(queryFor("noise"), "");
});

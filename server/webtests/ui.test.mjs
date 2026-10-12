import assert from "node:assert/strict";
import test from "node:test";

import { deviceLabel, fmtAgo, fmtBytes, fmtDur, fmtUptime } from "../camd/web/js/lib.js";
import { matches, reconcile } from "../camd/web/js/events.js";

const now = () => Date.now() / 1000;

test("durations", () => {
  assert.equal(fmtDur(null), "Grabando");
  assert.equal(fmtDur(0.2), "1 s");
  assert.equal(fmtDur(45), "45 s");
  assert.equal(fmtDur(61), "1:01");
});

test("relative time", () => {
  assert.equal(fmtAgo(now()), "hace un momento");
  assert.equal(fmtAgo(now() - 300), "hace 5 min");
  assert.equal(fmtAgo(now() - 7300), "hace 2 h");
  assert.equal(fmtAgo(now() - 3 * 86400), "hace 3 d");
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
  assert.equal(deviceLabel(""), "Dispositivo desconocido");
});

test("event filters", () => {
  const ev = { start: now() - 60, kind: "motion", keep: false };
  assert.ok(matches("all", ev) && matches("today", ev) && matches("week", ev));
  assert.ok(!matches("yesterday", ev) && !matches("clips", ev) && !matches("kept", ev));
  assert.ok(matches("clips", { ...ev, kind: "manual" }));
  assert.ok(matches("kept", { ...ev, keep: true }));
});

test("reconcile drops deleted events inside the fetched range and keeps older pages", () => {
  const e = (id, start) => ({ id, start });
  const current = [e(5, 500), e(4, 400), e(3, 300), e(2, 200), e(1, 100)];
  const fresh = [e(6, 600), e(5, 500), e(3, 300)];
  assert.deepEqual(reconcile(current, fresh, 3).map((x) => x.id), [6, 5, 3, 2, 1]);
  assert.deepEqual(reconcile(current, [e(5, 500)], 60).map((x) => x.id), [5]);
  assert.deepEqual(reconcile(current, [], 60), []);
});

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";

import es from "../camd/web/js/es.js";

const dir = new URL("../camd/web/js/", import.meta.url);
const files = readdirSync(dir).filter((f) => f.endsWith(".js"));
const source = (f) => readFileSync(new URL(f, dir), "utf8");
const views = ["live", "events", "photos", "settings"];

function exportsOf(text) {
  const names = new Set();
  for (const m of text.matchAll(/export\s+(?:async\s+)?(?:function|const|let|class)\s+([A-Za-z0-9_$]+)/g)) names.add(m[1]);
  for (const m of text.matchAll(/export\s*\{([^}]+)\}/g)) m[1].split(",").forEach((n) => names.add(n.trim().split(/\s+as\s+/).pop()));
  return names;
}

test("every named import resolves to a real export", () => {
  for (const file of files) {
    for (const m of source(file).matchAll(/import\s*\{([^}]+)\}\s*from\s*"\.\/([a-z]+\.js)(?:\?v=[0-9a-z.]+)?"/g)) {
      const target = exportsOf(source(m[2]));
      for (const name of m[1].split(",").map((n) => n.trim().split(/\s+as\s+/)[0]).filter(Boolean)) {
        assert.ok(target.has(name), `${file} imports ${name} which ${m[2]} does not export`);
      }
    }
  }
});

test("sources carry no build stamp and every view mounts", () => {
  for (const file of files) assert.doesNotMatch(source(file), /\.js\?v=/, `${file} is stamped`);
  for (const view of views) assert.match(source(`${view}.js`), /export function mount\(/);
});

test("every icon used in views exists", () => {
  const lib = source("lib.js");
  const table = lib.slice(lib.indexOf("export const ICONS"), lib.indexOf("export function h("));
  const defined = new Set([...table.matchAll(/^\s{2}([a-zA-Z]+):\s*svg/gm)].map((m) => m[1]));
  for (const file of files) {
    for (const m of source(file).matchAll(/icon\("([a-zA-Z]+)"/g)) assert.ok(defined.has(m[1]), `${file} uses missing icon ${m[1]}`);
    for (const m of source(file).matchAll(/icon\([^()]*\? "([a-zA-Z]+)" : "([a-zA-Z]+)"/g)) {
      assert.ok(defined.has(m[1]) && defined.has(m[2]), `${file} uses a missing icon`);
    }
  }
  for (const id of views) assert.ok(defined.has(id), `nav icon ${id}`);
});

test("every translatable string has a Spanish translation and none is stale", () => {
  const used = new Set();
  for (const file of files.filter((f) => f !== "es.js")) {
    const text = source(file);
    for (const m of text.matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"/g)) used.add(m[1]);
    for (const m of text.matchAll(/\bcount\([^"]*?,\s*"((?:[^"\\]|\\.)*)",\s*"((?:[^"\\]|\\.)*)"/g)) { used.add(m[1]); used.add(m[2]); }
  }
  const tables = ["live.js", "events.js", "main.js", "settings.js", "lib.js"].map(source).join("\n");
  for (const key of Object.keys(es).filter((k) => !k.startsWith("label:"))) {
    assert.ok(used.has(key) || tables.includes(`"${key}"`), `unused translation: ${key}`);
  }
  for (const key of used) assert.ok(key in es, `missing Spanish for: ${key}`);
});

test("every object the detector knows has a Spanish name", () => {
  const python = readFileSync(new URL("../camd/detect.py", import.meta.url), "utf8");
  const block = python.slice(python.indexOf("COCO_LABELS = ("), python.indexOf("DEFAULT_LABELS"));
  const labels = [...block.matchAll(/"([a-z ]+)"/g)].map((m) => m[1]);
  assert.equal(labels.length, 80);
  for (const label of labels) assert.ok(`label:${label}` in es, `no Spanish name for ${label}`);
});

test("English sources contain no Spanish", () => {
  for (const file of files.filter((f) => !["es.js", "text.js"].includes(f))) assert.doesNotMatch(source(file), /[áéíóúñ¿¡]/, `${file} has Spanish text`);
});

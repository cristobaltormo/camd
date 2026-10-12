import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";

const dir = new URL("../camd/web/js/", import.meta.url);
const files = readdirSync(dir).filter((f) => f.endsWith(".js"));
const source = (f) => readFileSync(new URL(f, dir), "utf8");

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

test("every module has one build stamp and every view mounts", () => {
  const stamps = new Set();
  for (const file of files) {
    for (const m of source(file).matchAll(/\.js\?v=([0-9a-z.]+)/g)) stamps.add(m[1]);
  }
  assert.equal(stamps.size, 1, `mixed build stamps: ${[...stamps]}`);
  for (const view of ["live", "events", "photos", "settings"]) assert.match(source(`${view}.js`), /export function mount\(/);
});

test("every icon used in views exists", () => {
  const icons = exportsOf(source("lib.js"));
  assert.ok(icons.has("ICONS"));
  const table = source("lib.js").slice(source("lib.js").indexOf("export const ICONS"), source("lib.js").indexOf("export function h("));
  const defined = new Set([...table.matchAll(/^\s{2}([a-zA-Z]+):\s*svg/gm)].map((m) => m[1]));
  for (const file of files) {
    for (const m of source(file).matchAll(/icon\("([a-zA-Z]+)"/g)) assert.ok(defined.has(m[1]), `${file} uses missing icon ${m[1]}`);
    for (const m of source(file).matchAll(/icon\(\w+ \? "([a-zA-Z]+)" : "([a-zA-Z]+)"/g)) { assert.ok(defined.has(m[1])); assert.ok(defined.has(m[2])); }
  }
  for (const id of ["live", "events", "photos", "settings"]) assert.ok(defined.has(id), `nav icon ${id}`);
});

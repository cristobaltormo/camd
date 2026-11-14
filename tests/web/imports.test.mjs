import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";

import es from "../../web/js/es.js";

const dir = new URL("../../web/js/", import.meta.url);
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
  const tables = ["live.js", "events.js", "main.js", "settings.js", "lib.js", "timeline.js"].map(source).join("\n");
  for (const key of Object.keys(es).filter((k) => !k.startsWith("label:"))) {
    assert.ok(used.has(key) || tables.includes(`"${key}"`), `unused translation: ${key}`);
  }
  for (const key of used) assert.ok(key in es, `missing Spanish for: ${key}`);
});

test("every object the detector knows has a Spanish name", () => {
  const rust = readFileSync(new URL("../../src/labels.rs", import.meta.url), "utf8");
  const block = rust.slice(rust.indexOf("pub const COCO_LABELS"), rust.indexOf("];"));
  const labels = [...block.matchAll(/"([a-z ]+)"/g)].map((m) => m[1]);
  assert.equal(labels.length, 80);
  for (const label of labels) assert.ok(`label:${label}` in es, `no Spanish name for ${label}`);
});

test("English sources contain no Spanish", () => {
  for (const file of files.filter((f) => !["es.js", "text.js"].includes(f))) assert.doesNotMatch(source(file), /[áéíóúñ¿¡]/, `${file} has Spanish text`);
});

function topLevelArguments(text, open) {
  let depth = 1;
  let quote = null;
  let out = "";
  for (let i = open + 1; i < text.length && depth > 0; i++) {
    const c = text[i];
    if (quote) { if (c === "\\") i++; else if (c === quote) quote = null; continue; }
    if (c === '"' || c === "'" || c === "`") { quote = c; continue; }
    if (c === "(" || c === "[" || c === "{") depth++;
    else if (c === ")" || c === "]" || c === "}") depth--;
    if (depth === 1 && c !== "(" && c !== "[" && c !== "{") out += c;
  }
  return out;
}

test("children passed straight to the DOM never include a literal null", () => {
  for (const file of files) {
    const text = source(file);
    for (const m of text.matchAll(/\.(?:replaceChildren|append|prepend)\(/g)) {
      const args = topLevelArguments(text, m.index + m[0].length - 1);
      const line = text.slice(0, m.index).split("\n").length;
      assert.doesNotMatch(args, /\b(?:null|undefined)\b/, `${file}:${line} passes null to the DOM; filter the children or build them with h()`);
    }
  }
});

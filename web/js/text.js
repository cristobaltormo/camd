import es from "./es.js";

const CATALOGS = { en: {}, es };
export const LANGUAGES = [["auto", "Auto"], ["en", "English"], ["es", "Español"]];

const storedPreference = () => {
  try { return localStorage.getItem("cam.language") || "auto"; } catch (_) { return "auto"; }
};

export function resolveLanguage(preference = storedPreference(), browser = globalThis.navigator?.language || "en") {
  if (CATALOGS[preference]) return preference;
  const base = String(browser).slice(0, 2).toLowerCase();
  return CATALOGS[base] ? base : "en";
}

let current = resolveLanguage();

export const language = () => current;
export const preference = storedPreference;
export const locale = () => (storedPreference() === "auto" ? globalThis.navigator?.language || current : current);

export function setLanguage(value) {
  try { localStorage.setItem("cam.language", value); } catch (_) {}
  current = resolveLanguage(value);
  if (globalThis.document) document.documentElement.lang = current;
}

export function t(key, values) {
  const text = CATALOGS[current][key] ?? key;
  return values ? text.replace(/\{(\w+)\}/g, (_, name) => values[name] ?? "") : text;
}

export const count = (n, one, many) => t(n === 1 ? one : many, { n });

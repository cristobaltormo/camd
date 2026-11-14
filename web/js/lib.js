import { locale, t } from "./text.js";

const svg = (body) =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

export const ICONS = {
  live: svg('<rect x="2.75" y="6" width="13" height="12" rx="2.5"/><path d="M15.75 10.25l5.5-3v9.5l-5.5-3"/>'),
  events: svg('<circle cx="12" cy="12" r="8.75"/><path d="M12 7.5V12l3 2"/>'),
  photos: svg('<rect x="3" y="4" width="18" height="16" rx="3"/><circle cx="9" cy="10" r="1.75"/><path d="M3.5 17.5l4.75-4.25 3.75 3.25 3-2.5 5.5 4.5"/>'),
  settings: svg('<path d="M4 7.5h10M18 7.5h2M4 16.5h2M10 16.5h10"/><circle cx="16" cy="7.5" r="2"/><circle cx="8" cy="16.5" r="2"/>'),
  expand: svg('<path d="M4.5 9V4.5H9M19.5 9V4.5H15M4.5 15v4.5H9M19.5 15v4.5H15"/>'),
  shrink: svg('<path d="M9 4.5V9H4.5M15 4.5V9h4.5M9 19.5V15H4.5M15 19.5V15h4.5"/>'),
  close: svg('<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>'),
  snap: svg('<path d="M4 8.25h3.25L9 5.75h6l1.75 2.5H20V19H4z"/><circle cx="12" cy="13.25" r="3.25"/>'),
  trash: svg('<path d="M4.5 7h15M9.5 7V4.5h5V7M6.5 7l.9 12.5h9.2L17.5 7"/>'),
  eyeOff: svg('<path d="M3.5 3.5l17 17M10.6 6.2A9.8 9.8 0 0112 6c6.4 0 10 6 10 6a17 17 0 01-3.2 3.9M6.5 7.6A17 17 0 002 12s3.6 6 10 6a9.6 9.6 0 004.2-1"/><path d="M9.9 9.9a3 3 0 004.2 4.2"/>'),
  bell: svg('<path d="M6.25 16.5V11a5.75 5.75 0 0111.5 0v5.5l1.5 1.75H4.75z"/><path d="M10 20.75h4"/>'),
  prev: svg('<path d="M14.5 5.5L8 12l6.5 6.5"/>'),
  next: svg('<path d="M9.5 5.5L16 12l-6.5 6.5"/>'),
  phone: svg('<rect x="7" y="2.75" width="10" height="18.5" rx="2.5"/><path d="M11 18.25h2"/>'),
  monitor: svg('<rect x="3" y="4.5" width="18" height="12" rx="2"/><path d="M8.5 20h7M12 16.5V20"/>'),
  key: svg('<circle cx="8" cy="15" r="4"/><path d="M11 12l8.5-8.5M16.5 6.5l2.5 2.5M14 9l2 2"/>'),
  film: svg('<rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="M10.25 9.5v5l4.25-2.5z"/>'),
  signal: svg('<path d="M3 9.5a13 13 0 0118 0M6 12.75a8.5 8.5 0 0112 0M9 16a4 4 0 016 0"/><circle cx="12" cy="19" r=".75" fill="currentColor"/>'),
  alert: svg('<path d="M12 4l9 15.5H3z"/><path d="M12 10v4M12 17v.01"/>'),
  check: svg('<path d="M5 12.5l4.5 4.5L19 7.5"/>'),
  camera: svg('<rect x="2.75" y="6" width="13" height="12" rx="2.5"/><path d="M15.75 10.25l5.5-3v9.5l-5.5-3"/>'),
  scan: svg('<path d="M4 8V5.5A1.5 1.5 0 015.5 4H8M16 4h2.5A1.5 1.5 0 0120 5.5V8M20 16v2.5a1.5 1.5 0 01-1.5 1.5H16M8 20H5.5A1.5 1.5 0 014 18.5V16"/><circle cx="12" cy="10" r="2.25"/><path d="M8 16.5c.6-1.9 2.1-2.9 4-2.9s3.4 1 4 2.9"/>'),
  disk: svg('<ellipse cx="12" cy="6" rx="8" ry="2.75"/><path d="M4 6v12c0 1.5 3.6 2.75 8 2.75s8-1.25 8-2.75V6M4 12c0 1.5 3.6 2.75 8 2.75s8-1.25 8-2.75"/>'),
  palette: svg('<circle cx="12" cy="12" r="8.75"/><path d="M12 3.25v17.5" /><path d="M12 3.25a8.75 8.75 0 010 17.5z" fill="currentColor" stroke="none"/>'),
  app: svg('<rect x="4" y="3" width="16" height="18" rx="3"/><path d="M12 7v7M9 11.5l3 3 3-3"/>'),
  presence: svg('<path d="M3.5 11L12 4l8.5 7"/><path d="M5.5 9.5V20h13V9.5"/><path d="M10 20v-5h4v5"/>'),
  record: svg('<circle cx="12" cy="12" r="5.5" fill="currentColor" stroke="none"/>'),
  stop: svg('<rect x="7" y="7" width="10" height="10" rx="2" fill="currentColor" stroke="none"/>'),
  pin: svg('<path d="M12 3.75l2.55 5.2 5.7.83-4.13 4.02.98 5.68L12 16.8l-5.1 2.68.98-5.68-4.13-4.02 5.7-.83z"/>'),
  pinOn: svg('<path d="M12 3.75l2.55 5.2 5.7.83-4.13 4.02.98 5.68L12 16.8l-5.1 2.68.98-5.68-4.13-4.02 5.7-.83z" fill="currentColor"/>'),
  share: svg('<path d="M12 15V3.75M8 7.5l4-3.75 4 3.75M6.5 11H5v9h14v-9h-1.5"/>'),
  cloud: svg('<path d="M7 18.5a4 4 0 01-.6-7.95A5.5 5.5 0 0117 9.5a4.5 4.5 0 01.5 9z"/>'),
  refresh: svg('<path d="M19.5 12a7.5 7.5 0 01-13.3 4.75M4.5 12A7.5 7.5 0 0117.8 7.25"/><path d="M18 3.5v4h-4M6 20.5v-4h4"/>'),
};

export function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v === undefined || v === null || v === false) continue;
    if (k === "class") el.className = v;
    else if (k === "text") el.textContent = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (["value", "checked", "disabled", "hidden", "selected"].includes(k)) el[k] = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat()) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

export function icon(name, cls) {
  const span = document.createElement("span");
  span.className = "ico" + (cls ? " " + cls : "");
  span.innerHTML = ICONS[name] || "";
  return span;
}

export class LoginRequired extends Error {}

const listeners = { offline: [], login: [], access: [] };
export const on = (event, fn) => listeners[event].push(fn);
const emit = (event, value) => listeners[event].forEach((fn) => fn(value));

let offline = false;
export const isOffline = () => offline;
function setOffline(value) {
  if (offline === value) return;
  offline = value;
  emit("offline", value);
}

const TIMEOUT_MS = 20000;

export async function api(path, opts = {}) {
  const init = { method: opts.method || "GET", credentials: "same-origin", headers: {} };
  if (opts.body !== undefined) {
    init.body = JSON.stringify(opts.body);
    init.headers["Content-Type"] = "application/json";
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), opts.timeout || TIMEOUT_MS);
  let res;
  try {
    res = await fetch(path, { ...init, redirect: "manual", signal: controller.signal });
  } catch (e) {
    if (e.name === "AbortError") throw new Error(t("The server is taking too long to answer"));
    setOffline(true);
    throw new Error(t("No connection to the server"));
  } finally {
    clearTimeout(timer);
  }
  if (res.type === "opaqueredirect") {
    emit("access");
    throw new LoginRequired();
  }
  setOffline(false);
  if (res.status === 401 && !path.startsWith("/api/auth/")) {
    emit("login");
    throw new LoginRequired();
  }
  if (!res.ok) {
    let detail = res.statusText;
    try { detail = (await res.json()).detail || detail; } catch (_) {}
    throw new Error(typeof detail === "string" ? friendly(detail) : t("Invalid data"));
  }
  return res.status === 204 ? null : res.json();
}

const MESSAGES = {
  "camera did not answer": "The camera is not answering",
  "camera not available": "The camera is not available right now",
  "no frame": "The camera is not sending images",
  "no clip in progress": "No clip is being recorded",
  "too many attempts": "Too many attempts, wait a moment",
  "login failed": "Sign-in failed",
  "cannot remove the last way to sign in": "You cannot remove the last way to sign in",
};
const friendly = (detail) => (MESSAGES[detail] ? t(MESSAGES[detail]) : /^photo limit/.test(detail) ? t("The photo limit has been reached") : detail);

export const store = {
  get(key, fallback) {
    try { const v = localStorage.getItem("cam." + key); return v === null ? fallback : JSON.parse(v); } catch (_) { return fallback; }
  },
  set(key, value) {
    try { localStorage.setItem("cam." + key, JSON.stringify(value)); } catch (_) {}
  },
};

export const b64uToBuf = (s) => {
  s = s.replace(/-/g, "+").replace(/_/g, "/");
  s += "=".repeat((4 - (s.length % 4)) % 4);
  return Uint8Array.from(atob(s), (c) => c.charCodeAt(0)).buffer;
};
export const bufToB64u = (buf) =>
  btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

export const fmtTime = (ts) => new Date(ts * 1000).toLocaleTimeString(locale(), { hour: "numeric", minute: "2-digit" });

export function fmtDay(ts) {
  const d = new Date(ts * 1000);
  const today = new Date();
  const yesterday = new Date();
  yesterday.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return t("Today");
  if (d.toDateString() === yesterday.toDateString()) return t("Yesterday");
  const label = d.toLocaleDateString(locale(), { weekday: "long", day: "numeric", month: "long" });
  return label.charAt(0).toUpperCase() + label.slice(1);
}

export function fmtDur(s) {
  if (s === null || s === undefined) return t("Recording");
  if (s < 60) return `${Math.max(1, Math.round(s))} s`;
  return `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, "0")}`;
}

export function fmtAgo(ts) {
  const s = Math.max(0, Date.now() / 1000 - ts);
  if (s < 60) return t("just now");
  if (s < 3600) return t("{n} min ago", { n: Math.floor(s / 60) });
  if (s < 86400) return t("{n} h ago", { n: Math.floor(s / 3600) });
  return t("{n} d ago", { n: Math.floor(s / 86400) });
}

export function fmtUptime(s) {
  if (!s) return "-";
  const d = Math.floor(s / 86400);
  const hours = Math.floor((s % 86400) / 3600);
  const minutes = Math.floor((s % 3600) / 60);
  return d ? `${d} d ${hours} h` : hours ? `${hours} h ${minutes} min` : `${minutes} min`;
}

export function fmtBytes(b) {
  if (b > 1024 ** 3) return (b / 1024 ** 3).toFixed(1) + " GB";
  if (b > 1024 ** 2) return Math.round(b / 1024 ** 2) + " MB";
  return Math.round(b / 1024) + " KB";
}

export function deviceLabel(ua) {
  if (!ua) return t("Unknown device");
  const os = /iPhone/.test(ua) ? "iPhone" : /iPad/.test(ua) ? "iPad" : /Android/.test(ua) ? "Android" : /Mac OS X/.test(ua) ? "Mac" : /Windows/.test(ua) ? "Windows" : /Linux/.test(ua) ? "Linux" : t("Device");
  const browser = /Edg\//.test(ua) ? "Edge" : /Firefox\//.test(ua) ? "Firefox" : /Chrome\//.test(ua) || /CriOS/.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : "";
  return browser ? `${os} · ${browser}` : os;
}

export function toast(text, tone, action, sticky = false) {
  let box = document.getElementById("toasts");
  if (!box) {
    box = h("div", { id: "toasts", "aria-live": "polite" });
    document.body.append(box);
  }
  const el = h("div", { class: "toast " + (tone || "") }, h("span", { text }), action ? h("button", { class: "toast-action", text: action[0], onclick: action[1] }) : null);
  box.append(el);
  if (!sticky) {
    const life = action ? 7000 : 2600;
    setTimeout(() => el.classList.add("out"), life);
    setTimeout(() => el.remove(), life + 400);
  }
  return el;
}

export function confirmDialog(title, text, okLabel, danger) {
  return new Promise((resolve) => {
    const done = (value) => { document.removeEventListener("keydown", onKey); overlay.remove(); resolve(value); };
    const onKey = (e) => { if (e.key === "Escape") done(false); };
    const overlay = h("div", { class: "modal center-modal", onclick: (e) => { if (e.target === overlay) done(false); } },
      h("div", { class: "dialog stack", role: "alertdialog", "aria-modal": "true" },
        h("h3", { text: title }),
        text ? h("p", { class: "muted", text }) : null,
        h("div", { class: "row end" },
          h("button", { class: "btn", text: t("Cancel"), onclick: () => done(false) }),
          h("button", { class: "btn " + (danger ? "danger-solid" : "primary"), text: okLabel, onclick: () => done(true) })
        )
      )
    );
    document.addEventListener("keydown", onKey);
    document.body.append(overlay);
    overlay.querySelector(".btn:last-child").focus();
  });
}

export const spinner = () => h("span", { class: "spinner", role: "status", "aria-label": t("Loading") });

export function stateText(text, tone) {
  return h("span", { class: "state" }, h("span", { class: "dot " + (tone || "") }), text);
}

export function dayGroup(count, ...kids) {
  const group = h("section", { class: "day-group" }, ...kids);
  group.style.setProperty("--count", String(count));
  return group;
}

export function emptyState(iconName, title, text, action) {
  return h("div", { class: "empty" }, icon(iconName, "empty-ico"), h("h3", { text: title }), text ? h("p", { text }) : null, action || null);
}

export const isIOS = () => /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
export const isStandalone = () => window.matchMedia("(display-mode: standalone)").matches || navigator.standalone === true;

export const prefs = {
  get photoDest() { return store.get("photoDest", "ask"); },
  set photoDest(v) { store.set("photoDest", v); },
  get clipSeconds() { return store.get("clipSeconds", 30); },
  set clipSeconds(v) { store.set("clipSeconds", v); },
};

export async function saveToDevice(source, filename) {
  let blob = source;
  if (typeof source === "string") {
    const res = await fetch(source, { credentials: "same-origin" });
    if (!res.ok) throw new Error(t("The file could not be downloaded"));
    blob = await res.blob();
  }
  const file = new File([blob], filename, { type: blob.type });
  if (navigator.canShare && navigator.canShare({ files: [file] })) {
    try {
      await navigator.share({ files: [file] });
      return "shared";
    } catch (e) {
      if (e.name === "AbortError") return "cancelled";
    }
  }
  const href = URL.createObjectURL(blob);
  const a = h("a", { href, download: filename });
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(href), 10000);
  return "downloaded";
}

export async function busy(button, fn) {
  if (button.disabled) return;
  button.disabled = true;
  try { return await fn(); } finally { button.disabled = false; }
}

export function choiceDialog(title, text, choices, remember) {
  return new Promise((resolve) => {
    let rememberBox = null;
    const done = (value) => {
      document.removeEventListener("keydown", onKey);
      overlay.remove();
      resolve(value ? { value, remember: !!(rememberBox && rememberBox.checked) } : null);
    };
    const onKey = (e) => { if (e.key === "Escape") done(null); };
    if (remember) rememberBox = h("input", { type: "checkbox", id: "remember-choice" });
    const overlay = h("div", { class: "modal center-modal", onclick: (e) => { if (e.target === overlay) done(null); } },
      h("div", { class: "dialog stack", role: "dialog", "aria-modal": "true" },
        h("h3", { text: title }),
        text ? h("p", { class: "muted", text }) : null,
        h("div", { class: "choices" }, choices.map(([id, label, hint, ic]) =>
          h("button", { class: "choice", onclick: () => done(id) }, icon(ic), h("span", {}, h("strong", { text: label }), h("span", { class: "muted", text: hint }))))),
        remember ? h("label", { class: "remember", for: "remember-choice" }, rememberBox, h("span", { text: t("Remember my choice") })) : null,
        h("div", { class: "row end" }, h("button", { class: "btn", text: t("Cancel"), onclick: () => done(null) }))
      )
    );
    document.addEventListener("keydown", onKey);
    document.body.append(overlay);
  });
}

export function watchState(s) {
  const cam = s.camera;
  if (!cam.enabled) return { tone: "warn", text: t("Camera disabled") };
  if (cam.standby) return { tone: "warn", text: t("Camera off") };
  if (!cam.online) return { tone: "bad", text: t("Camera has no signal") };
  if (s.recording) return { tone: "rec", text: t("Recording") };
  return s.armed ? { tone: "ok", text: t("Watching") } : { tone: "", text: t("Not watching") };
}

export const shareStatus = (s) => window.dispatchEvent(new CustomEvent("cam:status", { detail: s }));

export function cameraState(cam) {
  if (cam.enabled === false) return { text: t("Disabled"), tone: "warn" };
  if (cam.standby) return { text: t("Off (privacy)"), tone: "warn" };
  return cam.online ? { text: t("Connected"), tone: "ok" } : { text: t("No signal"), tone: "bad" };
}

export function labelName(label) {
  const translated = t(`label:${label}`);
  return translated === `label:${label}` ? label.charAt(0).toUpperCase() + label.slice(1) : translated;
}

export function detectionBox(item) {
  const [x, y, width, height] = item.box;
  const frame = h("div", { class: "box" + (item.label === "person" ? " person" : "") + (y < 0.07 ? " inset" : "") }, h("span", { text: `${labelName(item.label)} ${Math.round(item.confidence * 100)}%` }));
  Object.assign(frame.style, { left: x * 100 + "%", top: y * 100 + "%", width: width * 100 + "%", height: height * 100 + "%" });
  return frame;
}

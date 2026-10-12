const svg = (body) =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

export const ICONS = {
  live: svg('<path d="M15 10l5-3v10l-5-3z"/><rect x="2" y="6" width="13" height="12" rx="2.5"/>'),
  events: svg('<rect x="3" y="4" width="18" height="17" rx="3"/><path d="M3 10h18M8 2.5v3M16 2.5v3"/>'),
  settings: svg('<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/>'),
  expand: svg('<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>'),
  shrink: svg('<path d="M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5"/>'),
  close: svg('<path d="M6 6l12 12M18 6L6 18"/>'),
  download: svg('<path d="M12 4v11M7 11l5 5 5-5M5 20h14"/>'),
  snap: svg('<path d="M4 8h3l1.5-2.5h7L17 8h3v11H4z"/><circle cx="12" cy="13" r="3.2"/>'),
  trash: svg('<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v6M14 11v6"/>'),
  shield: svg('<path d="M12 3l8 3v6c0 4.5-3.2 7.8-8 9-4.8-1.2-8-4.5-8-9V6z"/><path d="M8.5 12l2.5 2.5 4.5-5"/>'),
  shieldOff: svg('<path d="M12 3l8 3v6c0 4.5-3.2 7.8-8 9-4.8-1.2-8-4.5-8-9V6z"/><path d="M9 9l6 6M15 9l-6 6"/>'),
  eyeOff: svg('<path d="M3 3l18 18M10.6 6.2A9.8 9.8 0 0112 6c6.4 0 10 6 10 6a17 17 0 01-3.2 3.9M6.5 7.6A17 17 0 002 12s3.6 6 10 6a9.6 9.6 0 004.2-1"/><path d="M9.9 9.9a3 3 0 004.2 4.2"/>'),
  bell: svg('<path d="M6 16V11a6 6 0 0112 0v5l1.5 2h-15z"/><path d="M10 21h4"/>'),
  prev: svg('<path d="M15 5l-7 7 7 7"/>'),
  next: svg('<path d="M9 5l7 7-7 7"/>'),
  phone: svg('<rect x="7" y="2.5" width="10" height="19" rx="2.5"/><path d="M11 18.5h2"/>'),
  monitor: svg('<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>'),
  key: svg('<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M16 7l3 3M14 9l2 2"/>'),
  film: svg('<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M10 9.5v5l4.5-2.5z"/>'),
  signal: svg('<path d="M4 20v-3M9 20v-7M14 20V9M19 20V4"/>'),
  logo: svg('<path d="M3 12s3.6-6.5 9-6.5S21 12 21 12s-3.6 6.5-9 6.5S3 12 3 12z"/><circle cx="12" cy="12" r="3"/>'),
  moon: svg('<path d="M20 14.5A8 8 0 019.5 4 8 8 0 1020 14.5z"/>'),
  sun: svg('<circle cx="12" cy="12" r="4"/><path d="M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4"/>'),
  check: svg('<path d="M5 12.5l4.5 4.5L19 7.5"/>'),
  camera: svg('<rect x="3" y="7" width="18" height="12" rx="3"/><circle cx="12" cy="13" r="3.2"/><path d="M9 7l1-2h4l1 2"/>'),
  sliders: svg('<path d="M5 4v16M12 4v16M19 4v16"/><circle cx="5" cy="9" r="2" fill="currentColor"/><circle cx="12" cy="15" r="2" fill="currentColor"/><circle cx="19" cy="8" r="2" fill="currentColor"/>'),
  disk: svg('<ellipse cx="12" cy="6" rx="8" ry="3"/><path d="M4 6v6c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6"/>'),
  users: svg('<circle cx="12" cy="8" r="3.5"/><path d="M5 20c.5-3.6 3.2-5.5 7-5.5s6.5 1.9 7 5.5"/>'),
  palette: svg('<path d="M12 3a9 9 0 100 18c1.4 0 2-.9 2-1.8 0-1.2-1-1.5-1-2.7 0-1 .8-1.5 1.8-1.5H17a4 4 0 004-4c0-4.4-4-8-9-8z"/><circle cx="7.5" cy="11" r="1" fill="currentColor"/><circle cx="10.5" cy="7" r="1" fill="currentColor"/><circle cx="15" cy="7.5" r="1" fill="currentColor"/>'),
  app: svg('<rect x="4" y="3" width="16" height="18" rx="3"/><path d="M12 7v7M9 11.5l3 3 3-3"/>'),
  clock: svg('<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>'),
  presence: svg('<path d="M3 11l9-7 9 7"/><path d="M5 10v10h14V10"/><path d="M10 20v-5h4v5"/>'),
  more: svg('<circle cx="5" cy="12" r="1.2" fill="currentColor"/><circle cx="12" cy="12" r="1.2" fill="currentColor"/><circle cx="19" cy="12" r="1.2" fill="currentColor"/>'),
  select: svg('<rect x="4" y="4" width="16" height="16" rx="3"/><path d="M8.5 12.5l2.5 2.5 4.5-5"/>'),
  keyboard: svg('<rect x="2.5" y="6" width="19" height="12" rx="2.5"/><path d="M6.5 10h.01M10 10h.01M13.5 10h.01M17 10h.01M7 14h10"/>'),
  photos: svg('<rect x="3" y="4" width="18" height="16" rx="3"/><circle cx="9" cy="10" r="1.8"/><path d="M4 18l5-5 4 4 3-3 4 4"/>'),
  record: svg('<circle cx="12" cy="12" r="8"/><circle cx="12" cy="12" r="3.5" fill="currentColor"/>'),
  stop: svg('<rect x="6" y="6" width="12" height="12" rx="2.5" fill="currentColor"/>'),
  pin: svg('<path d="M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1L3.2 9.5l6.1-.9z"/>'),
  pinOn: svg('<path d="M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1L3.2 9.5l6.1-.9z" fill="currentColor"/>'),
  share: svg('<path d="M12 15V4M8 8l4-4 4 4M5 13v6h14v-6"/>'),
  cloud: svg('<path d="M7 18a4 4 0 01-.6-7.95A5.5 5.5 0 0117 9a4.5 4.5 0 01.5 9z"/>'),
  refresh: svg('<path d="M20 11a8 8 0 00-14-4.5L4 9M4 4v5h5M4 13a8 8 0 0014 4.5L20 15M20 20v-5h-5"/>'),
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
    if (e.name === "AbortError") throw new Error("El servidor tarda demasiado en responder");
    setOffline(true);
    throw new Error("Sin conexión con el servidor");
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
    throw new Error(typeof detail === "string" ? friendly(detail) : "Datos no válidos");
  }
  return res.status === 204 ? null : res.json();
}

const MESSAGES = {
  "camera did not answer": "La cámara no responde",
  "camera not available": "La cámara no está disponible ahora",
  "no frame": "La cámara no está enviando imagen",
  "no clip in progress": "No hay ningún clip grabándose",
  "too many attempts": "Demasiados intentos, espera un momento",
};
const friendly = (detail) => MESSAGES[detail] || (/^photo limit/.test(detail) ? "Has llegado al límite de fotos guardadas" : detail);

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

export const fmtTime = (ts) => new Date(ts * 1000).toLocaleTimeString("es-ES", { hour: "2-digit", minute: "2-digit" });

export function fmtDay(ts) {
  const d = new Date(ts * 1000);
  const today = new Date();
  const y = new Date();
  y.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return "Hoy";
  if (d.toDateString() === y.toDateString()) return "Ayer";
  const label = d.toLocaleDateString("es-ES", { weekday: "long", day: "numeric", month: "long" });
  return label.charAt(0).toUpperCase() + label.slice(1);
}

export function fmtDur(s) {
  if (s === null || s === undefined) return "Grabando";
  if (s < 60) return `${Math.max(1, Math.round(s))} s`;
  return `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, "0")}`;
}

export function fmtAgo(ts) {
  const s = Math.max(0, Date.now() / 1000 - ts);
  if (s < 60) return "hace un momento";
  if (s < 3600) return `hace ${Math.floor(s / 60)} min`;
  if (s < 86400) return `hace ${Math.floor(s / 3600)} h`;
  return `hace ${Math.floor(s / 86400)} d`;
}

export function fmtUptime(s) {
  if (!s) return "-";
  const d = Math.floor(s / 86400);
  const hh = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  return d ? `${d} d ${hh} h` : hh ? `${hh} h ${m} min` : `${m} min`;
}

export function fmtBytes(b) {
  if (b > 1024 ** 3) return (b / 1024 ** 3).toFixed(1) + " GB";
  if (b > 1024 ** 2) return Math.round(b / 1024 ** 2) + " MB";
  return Math.round(b / 1024) + " KB";
}

export function deviceLabel(ua) {
  if (!ua) return "Dispositivo desconocido";
  const os = /iPhone/.test(ua) ? "iPhone" : /iPad/.test(ua) ? "iPad" : /Android/.test(ua) ? "Android" : /Mac OS X/.test(ua) ? "Mac" : /Windows/.test(ua) ? "Windows" : /Linux/.test(ua) ? "Linux" : "Dispositivo";
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
          h("button", { class: "btn", text: "Cancelar", onclick: () => done(false) }),
          h("button", { class: "btn " + (danger ? "danger-solid" : "primary"), text: okLabel, onclick: () => done(true) })
        )
      )
    );
    document.addEventListener("keydown", onKey);
    document.body.append(overlay);
    overlay.querySelector(".btn:last-child").focus();
  });
}

export const spinner = () => h("span", { class: "spinner", role: "status", "aria-label": "Cargando" });

export function chip(text, tone) {
  return h("span", { class: "chip" }, h("span", { class: "dot " + (tone || "") }), text);
}

export function emptyState(iconName, title, text, action) {
  return h("div", { class: "empty" }, icon(iconName), h("h3", { text: title }), text ? h("p", { class: "muted", text }) : null, action || null);
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
    if (!res.ok) throw new Error("No se pudo descargar el archivo");
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
        remember ? h("label", { class: "remember", for: "remember-choice" }, rememberBox, h("span", { text: "Recordar mi elección" })) : null,
        h("div", { class: "row end" }, h("button", { class: "btn", text: "Cancelar", onclick: () => done(null) }))
      )
    );
    document.addEventListener("keydown", onKey);
    document.body.append(overlay);
  });
}

export function cameraState(cam) {
  if (cam.enabled === false) return { text: "Desactivada", tone: "warn" };
  if (cam.standby) return { text: "Apagada (privacidad)", tone: "warn" };
  return cam.online ? { text: "Conectada", tone: "ok" } : { text: "Sin señal", tone: "bad" };
}

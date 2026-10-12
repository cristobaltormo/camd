"use strict";

const app = document.getElementById("app");
const state = {
  tab: "live",
  timers: [],
  rects: [],
  offline: false,
  events: [],
  eventsDone: false,
  stop: null,
};

const svg = (body) =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

const ICONS = {
  live: svg('<path d="M15 10l5-3v10l-5-3z"/><rect x="2" y="6" width="13" height="12" rx="2.5"/>'),
  events: svg('<rect x="3" y="4" width="18" height="17" rx="3"/><path d="M3 10h18M8 2.5v3M16 2.5v3"/>'),
  settings: svg('<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/>'),
  expand: svg('<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>'),
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
  key: svg('<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M16 7l3 3M14 9l2 2"/>'),
  film: svg('<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M10 9.5v5l4.5-2.5z"/>'),
  signal: svg('<path d="M4 20v-3M9 20v-7M14 20V9M19 20V4"/>'),
  logo: svg('<path d="M3 12s3.6-6.5 9-6.5S21 12 21 12s-3.6 6.5-9 6.5S3 12 3 12z"/><circle cx="12" cy="12" r="3"/>'),
};

function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v === undefined || v === null || v === false) continue;
    if (k === "class") el.className = v;
    else if (k === "text") el.textContent = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "value" || k === "checked" || k === "disabled" || k === "hidden") el[k] = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat()) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

function icon(name, cls) {
  const span = document.createElement("span");
  span.className = "ico" + (cls ? " " + cls : "");
  span.innerHTML = ICONS[name];
  return span;
}

class LoginRequired extends Error {}

async function api(path, opts = {}) {
  const init = { method: opts.method || "GET", credentials: "same-origin", headers: {} };
  if (opts.body !== undefined) {
    init.body = JSON.stringify(opts.body);
    init.headers["Content-Type"] = "application/json";
  }
  let res;
  try {
    res = await fetch(path, init);
  } catch (e) {
    setOffline(true);
    throw new Error("Sin conexión con el servidor");
  }
  setOffline(false);
  if (res.status === 401 && !path.startsWith("/api/auth/")) {
    clearTimers();
    showAuth();
    throw new LoginRequired();
  }
  if (!res.ok) {
    let detail = res.statusText;
    try { detail = (await res.json()).detail || detail; } catch (_) {}
    throw new Error(typeof detail === "string" ? detail : "Error");
  }
  return res.status === 204 ? null : res.json();
}

function setOffline(value) {
  if (state.offline === value) return;
  state.offline = value;
  const banner = document.getElementById("offline");
  if (banner) banner.hidden = !value;
}

function clearTimers() {
  state.timers.forEach(clearInterval);
  state.timers = [];
  if (state.stop) { state.stop(); state.stop = null; }
}

function every(ms, fn) {
  state.timers.push(setInterval(fn, ms));
}

const b64uToBuf = (s) => {
  s = s.replace(/-/g, "+").replace(/_/g, "/");
  s += "=".repeat((4 - (s.length % 4)) % 4);
  return Uint8Array.from(atob(s), (c) => c.charCodeAt(0)).buffer;
};
const bufToB64u = (buf) =>
  btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

function fmtTime(ts) {
  return new Date(ts * 1000).toLocaleTimeString("es-ES", { hour: "2-digit", minute: "2-digit" });
}

function fmtDay(ts) {
  const d = new Date(ts * 1000);
  const today = new Date();
  const y = new Date();
  y.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return "Hoy";
  if (d.toDateString() === y.toDateString()) return "Ayer";
  const label = d.toLocaleDateString("es-ES", { weekday: "long", day: "numeric", month: "long" });
  return label.charAt(0).toUpperCase() + label.slice(1);
}

function fmtDur(s) {
  if (s === null || s === undefined) return "Grabando";
  if (s < 60) return `${Math.max(1, Math.round(s))} s`;
  return `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, "0")}`;
}

function fmtAgo(ts) {
  const s = Math.max(0, Date.now() / 1000 - ts);
  if (s < 60) return "hace un momento";
  if (s < 3600) return `hace ${Math.floor(s / 60)} min`;
  if (s < 86400) return `hace ${Math.floor(s / 3600)} h`;
  return `hace ${Math.floor(s / 86400)} d`;
}

function fmtUptime(s) {
  if (!s) return "-";
  const d = Math.floor(s / 86400);
  const hh = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  return d ? `${d} d ${hh} h` : hh ? `${hh} h ${m} min` : `${m} min`;
}

function fmtBytes(b) {
  if (b > 1024 ** 3) return (b / 1024 ** 3).toFixed(1) + " GB";
  if (b > 1024 ** 2) return Math.round(b / 1024 ** 2) + " MB";
  return Math.round(b / 1024) + " KB";
}

function deviceLabel(ua) {
  if (!ua) return "Dispositivo desconocido";
  const os = /iPhone/.test(ua) ? "iPhone" : /iPad/.test(ua) ? "iPad" : /Android/.test(ua) ? "Android" : /Mac OS X/.test(ua) ? "Mac" : /Windows/.test(ua) ? "Windows" : /Linux/.test(ua) ? "Linux" : "Dispositivo";
  const browser = /Edg\//.test(ua) ? "Edge" : /Firefox\//.test(ua) ? "Firefox" : /Chrome\//.test(ua) || /CriOS/.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : "";
  return browser ? `${os} · ${browser}` : os;
}

// ---------- feedback ----------

function toast(text, tone) {
  let box = document.getElementById("toasts");
  if (!box) {
    box = h("div", { id: "toasts", "aria-live": "polite" });
    document.body.append(box);
  }
  const el = h("div", { class: "toast " + (tone || "") }, text);
  box.append(el);
  setTimeout(() => el.classList.add("out"), 2600);
  setTimeout(() => el.remove(), 3000);
}

function confirmDialog(title, text, okLabel, danger) {
  return new Promise((resolve) => {
    const done = (value) => { overlay.remove(); resolve(value); };
    const overlay = h("div", { class: "modal center-modal", onclick: (e) => { if (e.target === overlay) done(false); } },
      h("div", { class: "dialog stack", role: "alertdialog" },
        h("h3", { text: title }),
        text ? h("p", { class: "muted", text }) : null,
        h("div", { class: "row end" },
          h("button", { class: "btn", text: "Cancelar", onclick: () => done(false) }),
          h("button", { class: "btn " + (danger ? "danger-solid" : "primary"), text: okLabel, onclick: () => done(true) })
        )
      )
    );
    document.body.append(overlay);
  });
}

function spinner() {
  return h("span", { class: "spinner", role: "status", "aria-label": "Cargando" });
}

// ---------- authentication ----------

async function registerPasskey(code) {
  const options = await api("/api/auth/register/options", { method: "POST", body: { code } });
  options.challenge = b64uToBuf(options.challenge);
  options.user.id = b64uToBuf(options.user.id);
  (options.excludeCredentials || []).forEach((c) => (c.id = b64uToBuf(c.id)));
  const cred = await navigator.credentials.create({ publicKey: options });
  await api("/api/auth/register/verify", {
    method: "POST",
    body: {
      code,
      name: deviceLabel(navigator.userAgent).slice(0, 40),
      credential: {
        id: cred.id,
        rawId: bufToB64u(cred.rawId),
        type: cred.type,
        response: {
          clientDataJSON: bufToB64u(cred.response.clientDataJSON),
          attestationObject: bufToB64u(cred.response.attestationObject),
          transports: cred.response.getTransports ? cred.response.getTransports() : [],
        },
        clientExtensionResults: cred.getClientExtensionResults(),
      },
    },
  });
}

async function loginPasskey() {
  const { cid, options } = await api("/api/auth/login/options", { method: "POST" });
  options.challenge = b64uToBuf(options.challenge);
  (options.allowCredentials || []).forEach((c) => (c.id = b64uToBuf(c.id)));
  const cred = await navigator.credentials.get({ publicKey: options });
  await api("/api/auth/login/verify", {
    method: "POST",
    body: {
      cid,
      credential: {
        id: cred.id,
        rawId: bufToB64u(cred.rawId),
        type: cred.type,
        response: {
          clientDataJSON: bufToB64u(cred.response.clientDataJSON),
          authenticatorData: bufToB64u(cred.response.authenticatorData),
          signature: bufToB64u(cred.response.signature),
          userHandle: cred.response.userHandle ? bufToB64u(cred.response.userHandle) : null,
        },
        clientExtensionResults: cred.getClientExtensionResults(),
      },
    },
  });
}

function authError(e) {
  if (e && (e.name === "NotAllowedError" || e.name === "AbortError")) return "Operación cancelada.";
  if (e && e.name === "InvalidStateError") return "Este dispositivo ya tiene una passkey registrada.";
  return (e && e.message) || String(e);
}

async function showAuth() {
  clearTimers();
  document.body.classList.add("auth");
  let st;
  try {
    st = await api("/api/auth/state");
  } catch (e) {
    app.replaceChildren(offlineScreen());
    return;
  }
  if (st.logged_in) return showMain();

  if (!window.PublicKeyCredential) {
    app.replaceChildren(h("div", { class: "auth-wrap" }, h("div", { class: "auth-head" }, icon("logo", "logo"), h("h1", { text: "Habitación" }), h("p", { class: "muted", text: "Este navegador no admite passkeys." }))));
    return;
  }

  const err = h("div", { class: "err", role: "alert" });
  const busy = (btn, label, fn) => async () => {
    err.textContent = "";
    const prev = btn.textContent;
    btn.disabled = true;
    btn.textContent = label;
    try { await fn(); } catch (e) { err.textContent = authError(e); } finally { btn.disabled = false; btn.textContent = prev; }
  };

  const loginBtn = h("button", { class: "btn primary block big", text: "Entrar con passkey" });
  loginBtn.addEventListener("click", busy(loginBtn, "Verificando...", async () => { await loginPasskey(); await showMain(); }));

  const codeInput = h("input", { type: "text", placeholder: "Código de alta", autocomplete: "off", autocapitalize: "off", spellcheck: "false", inputmode: "text", "aria-label": "Código de alta" });
  const enrollBtn = h("button", { class: "btn block " + (st.enrolled ? "" : "primary big"), text: "Registrar este dispositivo" });
  enrollBtn.addEventListener("click", busy(enrollBtn, "Registrando...", async () => {
    const code = codeInput.value.trim();
    if (!code) throw new Error("Introduce el código de alta.");
    await registerPasskey(code);
    await showMain();
  }));
  const enrollBox = h("div", { class: "stack", hidden: st.enrolled },
    h("p", { class: "muted", text: "Pide un código de alta desde un dispositivo ya registrado, o genéralo en el servidor." }),
    codeInput, enrollBtn
  );

  const toggle = st.enrolled
    ? h("button", { class: "link", text: "Registrar un dispositivo nuevo", onclick: (e) => { enrollBox.hidden = false; e.currentTarget.hidden = true; codeInput.focus(); } })
    : null;

  app.replaceChildren(
    h("div", { class: "auth-wrap" },
      h("div", { class: "auth-head" },
        icon("logo", "logo"),
        h("h1", { text: "Habitación" }),
        h("p", { class: "muted", text: st.enrolled ? "Acceso protegido con passkey" : "Primer acceso: registra este dispositivo" })
      ),
      h("div", { class: "card stack" }, st.enrolled ? loginBtn : null, enrollBox, toggle, err)
    )
  );
}

function offlineScreen() {
  return h("div", { class: "auth-wrap" },
    h("div", { class: "auth-head" }, icon("logo", "logo"), h("h1", { text: "Habitación" }), h("p", { class: "muted", text: "No se puede conectar con el servidor." })),
    h("div", { class: "card stack" }, h("button", { class: "btn primary block", text: "Reintentar", onclick: () => location.reload() }))
  );
}

// ---------- shell ----------

const TABS = [["live", "En vivo"], ["events", "Eventos"], ["settings", "Ajustes"]];

async function showMain() {
  clearTimers();
  document.body.classList.remove("auth");
  const body = h("div", { class: "page" });
  const banner = h("div", { id: "offline", class: "banner", hidden: !state.offline, role: "status" },
    h("span", { text: "Sin conexión con el servidor" }),
    h("button", { class: "link", text: "Reintentar", onclick: () => location.reload() })
  );
  const nav = h("nav", { class: "tabs", "aria-label": "Secciones" },
    h("div", {}, TABS.map(([id, label]) =>
      h("button", {
        class: state.tab === id ? "on" : "",
        "aria-current": state.tab === id ? "page" : null,
        onclick: () => { if (state.tab !== id) { state.tab = id; showMain(); } },
      }, icon(id), h("span", { text: label }))
    ))
  );
  app.replaceChildren(banner, body, nav);
  body.append(h("div", { class: "loading" }, spinner()));
  try {
    const fresh = h("div");
    if (state.tab === "live") await renderLive(fresh);
    else if (state.tab === "events") await renderEvents(fresh);
    else await renderSettings(fresh);
    body.replaceChildren(...fresh.childNodes);
  } catch (e) {
    if (e instanceof LoginRequired) return;
    body.replaceChildren(h("div", { class: "empty" }, icon("signal"), h("p", { text: e.message }), h("button", { class: "btn", text: "Reintentar", onclick: () => showMain() })));
  }
  const m = location.hash.match(/event=(\d+)/);
  if (m) {
    history.replaceState(null, "", location.pathname);
    openEvent(Number(m[1]));
  }
}

function pageTitle(text, ...actions) {
  return h("header", { class: "top" }, h("h1", { text }), h("div", { class: "row tight" }, actions));
}

function chip(text, tone) {
  return h("span", { class: "chip" }, h("span", { class: "dot " + (tone || "") }), text);
}

const REASONS = {
  privacy: "Privacidad activa: la cámara está apagada",
  snoozed: "Avisos en pausa",
  manual: "Modo manual",
  home: "Estás en casa: no se graba",
  away: "Fuera de casa: vigilando",
  "auto-without-device": "Sin dispositivo de presencia: no se graba",
};

// ---------- live ----------

async function renderLive(body) {
  const img = h("img", { alt: "Imagen en directo", draggable: "false" });
  const overlay = h("div", { class: "overlay" });
  const liveBadge = h("span", { class: "badge live", hidden: true }, h("span", { class: "pulse" }), "EN VIVO");
  const recBadge = h("span", { class: "badge rec", hidden: true }, h("span", { class: "pulse" }), "GRABANDO");
  const fullBtn = h("button", { class: "round", "aria-label": "Pantalla completa", onclick: () => toggleFull() }, icon("expand"));
  const snapBtn = h("a", { class: "round", "aria-label": "Guardar foto", href: "/api/snapshot?download=1" }, icon("snap"));
  const viewer = h("div", { class: "viewer" }, img, overlay, h("div", { class: "badges" }, liveBadge, recBadge), h("div", { class: "viewer-actions" }, snapBtn, fullBtn));

  function toggleFull() {
    const on = viewer.classList.toggle("full");
    document.body.classList.toggle("noscroll", on);
    fullBtn.replaceChildren(icon(on ? "close" : "expand"));
  }

  const hero = h("div", { class: "card hero" });
  const controls = h("div", { class: "stack" });
  const last = h("div");
  body.append(pageTitle("Habitación"), viewer, hero, controls, last);

  let streaming = false;
  let standby = false;
  let watcher = null;

  const showOverlay = (kind, text, action) => {
    overlay.hidden = false;
    liveBadge.hidden = true;
    overlay.replaceChildren(
      kind === "wait" ? spinner() : icon(kind === "standby" ? "eyeOff" : "signal"),
      h("div", { text }),
      action ? h("button", { class: "btn", text: action[0], onclick: action[1] }) : null
    );
  };

  const stopStream = () => {
    streaming = false;
    clearInterval(watcher);
    img.removeAttribute("src");
  };

  const startStream = () => {
    if (streaming || standby) return;
    streaming = true;
    showOverlay("wait", "Conectando...");
    img.src = "/api/live?t=" + Date.now();
    clearInterval(watcher);
    watcher = setInterval(() => {
      if (img.naturalWidth > 0) {
        overlay.hidden = true;
        liveBadge.hidden = false;
        clearInterval(watcher);
      }
    }, 250);
  };

  img.addEventListener("load", () => { overlay.hidden = true; liveBadge.hidden = false; });
  img.addEventListener("error", () => {
    if (!streaming) return;
    stopStream();
    showOverlay("signal", "Sin imagen", ["Reintentar", startStream]);
  });

  const onVisible = () => {
    if (document.hidden) stopStream();
    else if (!standby) startStream();
  };
  document.addEventListener("visibilitychange", onVisible);
  state.stop = () => {
    document.removeEventListener("visibilitychange", onVisible);
    stopStream();
    document.body.classList.remove("noscroll");
  };

  async function refresh() {
    const s = await api("/api/status");
    standby = s.camera.standby;
    recBadge.hidden = !s.recording;
    renderHero(hero, s);
    controls.replaceChildren(controlsCard(s, refresh));
    last.replaceChildren(lastEvent(s));
    if (standby) {
      stopStream();
      showOverlay("standby", "Cámara apagada por privacidad", ["Encender", async () => { try { await api("/api/privacy", { method: "POST", body: { on: false } }); refresh(); } catch (e) { toast(e.message, "bad"); } }]);
    } else if (!streaming && !document.hidden) {
      startStream();
    }
  }
  await refresh();
  every(4000, () => refresh().catch(() => {}));
}

function renderHero(hero, s) {
  const ok = s.armed;
  hero.className = "card hero " + (ok ? "on" : "off");
  const camTone = s.camera.standby ? "warn" : s.camera.online ? "ok" : "bad";
  const camText = s.camera.standby ? "Cámara apagada" : s.camera.online ? "Cámara conectada" : "Cámara sin señal";
  hero.replaceChildren(
    h("div", { class: "hero-main" },
      icon(ok ? "shield" : "shieldOff", "hero-ico"),
      h("div", {},
        h("div", { class: "hero-title", text: ok ? "Vigilando" : "No vigila" }),
        h("div", { class: "muted", text: REASONS[s.reason] || s.reason })
      )
    ),
    h("div", { class: "chips" },
      chip(camText, camTone),
      s.motion && !s.recording ? chip("Movimiento", "warn") : null,
      s.camera.rssi ? chip(`${s.camera.rssi} dBm`) : null
    )
  );
}

function controlsCard(s, refresh) {
  const modes = [["auto", "Auto"], ["armed", "Armado"], ["disarmed", "Desarmado"]];
  const run = (fn) => async () => { try { await fn(); } catch (e) { toast(e.message, "bad"); } refresh(); };
  const seg = h("div", { class: "seg", role: "group", "aria-label": "Modo" }, modes.map(([id, label]) =>
    h("button", { class: s.mode === id ? "on" : "", text: label, "aria-pressed": s.mode === id ? "true" : "false", onclick: run(() => api("/api/mode", { method: "POST", body: { mode: id } })) })
  ));

  const snoozed = s.snooze_until * 1000 > Date.now();
  const snooze = h("div", { class: "row" },
    h("div", {}, h("div", { text: "Pausar avisos" }), h("div", { class: "muted", text: snoozed ? "En pausa hasta las " + fmtTime(s.snooze_until) : "No se grabará ni se avisará" })),
    h("div", { class: "row tight" },
      snoozed
        ? h("button", { class: "btn small", text: "Reanudar", onclick: run(() => api("/api/snooze", { method: "POST", body: { minutes: 0 } })) })
        : [[60, "1 h"], [480, "8 h"]].map(([m, l]) => h("button", { class: "btn small", text: l, onclick: run(() => api("/api/snooze", { method: "POST", body: { minutes: m } })) }))
    )
  );

  const privacy = h("div", { class: "row" },
    h("div", {}, h("div", { text: "Privacidad" }), h("div", { class: "muted", text: "Apaga el sensor de la cámara" })),
    h("input", { type: "checkbox", class: "switch", "aria-label": "Privacidad", checked: s.privacy, onchange: async (e) => {
      try { await api("/api/privacy", { method: "POST", body: { on: e.target.checked } }); } catch (err) { toast(err.message, "bad"); }
      refresh();
    } })
  );
  const hint = !s.presence.configured && s.mode === "auto" ? h("div", { class: "muted", text: "Modo auto sin dispositivo configurado: no se graba hasta que lo armes." }) : null;
  return h("div", { class: "card stack" }, seg, hint, h("hr"), snooze, h("hr"), privacy);
}

function lastEvent(s) {
  const ev = s.last_event;
  if (!ev) return h("div");
  return h("button", { class: "card link-card", onclick: () => openEvent(ev.id) },
    icon("film"),
    h("div", { class: "grow" }, h("div", { text: "Último movimiento" }), h("div", { class: "muted", text: `${fmtDay(ev.start_ts)}, ${fmtTime(ev.start_ts)} · ${fmtAgo(ev.start_ts)}` })),
    icon("next", "dim")
  );
}

// ---------- events ----------

async function loadEvents(reset) {
  const before = !reset && state.events.length ? state.events[state.events.length - 1].start : null;
  const { events } = await api("/api/events?limit=60" + (before ? `&before=${before}` : ""));
  state.events = reset ? events : state.events.concat(events);
  state.eventsDone = events.length < 60;
}

async function renderEvents(body) {
  await loadEvents(true);
  body.append(pageTitle("Eventos"));
  const list = h("div");
  body.append(list);
  const more = h("button", { class: "btn block more", text: "Cargar más" });
  more.addEventListener("click", async () => {
    more.disabled = true;
    try { await loadEvents(false); drawEvents(list); } catch (e) { toast(e.message, "bad"); }
    more.disabled = false;
    more.hidden = state.eventsDone;
  });
  body.append(more);
  more.hidden = state.eventsDone;
  drawEvents(list);
}

function drawEvents(list) {
  list.replaceChildren();
  if (!state.events.length) {
    list.append(h("div", { class: "empty" }, icon("film"), h("p", { text: "Sin eventos todavía" }), h("p", { class: "muted", text: "Aquí aparecerán las grabaciones cuando se detecte movimiento." })));
    return;
  }
  let lastDay = "";
  for (const ev of state.events) {
    const day = fmtDay(ev.start);
    if (day !== lastDay) { list.append(h("div", { class: "day", text: day })); lastDay = day; }
    const failed = ev.status === "failed";
    const thumb = ev.status === "done"
      ? h("div", { class: "thumb" }, h("img", { src: `/api/events/${ev.id}/thumb`, alt: "", loading: "lazy" }), h("span", { class: "dur", text: fmtDur(ev.duration) }))
      : h("div", { class: "thumb empty-thumb" }, icon(failed ? "signal" : "film"));
    list.append(h("button", { class: "event", onclick: () => openEvent(ev.id) },
      thumb,
      h("div", { class: "meta" },
        h("strong", { text: fmtTime(ev.start) }),
        h("span", { class: "muted", text: failed ? "Error al guardar" : ev.status === "recording" ? "Grabando ahora" : fmtAgo(ev.start) })
      ),
      icon("next", "dim")
    ));
  }
}

async function openEvent(id) {
  const index = () => state.events.findIndex((e) => e.id === id);
  const modal = h("div", { class: "modal sheet-modal", onclick: (e) => { if (e.target === modal) close(); } });
  const onKey = (e) => {
    if (e.key === "Escape") close();
    else if (e.key === "ArrowLeft") go(1);
    else if (e.key === "ArrowRight") go(-1);
  };
  const close = () => { document.removeEventListener("keydown", onKey); modal.remove(); document.body.classList.remove("noscroll"); };
  document.addEventListener("keydown", onKey);
  document.body.classList.add("noscroll");

  function go(step) {
    const i = index();
    if (i < 0) return;
    const next = state.events[i + step];
    if (next && next.status === "done") { id = next.id; draw(); }
  }

  function draw() {
    const i = index();
    const ev = i >= 0 ? state.events[i] : null;
    const older = ev && state.events[i + 1] && state.events[i + 1].status === "done";
    const newer = ev && state.events[i - 1] && state.events[i - 1].status === "done";
    const video = h("video", { controls: true, playsinline: true, autoplay: true, src: `/api/events/${id}/clip`, poster: `/api/events/${id}/thumb` });
    modal.replaceChildren(
      h("div", { class: "sheet stack" },
        h("div", { class: "row" },
          h("div", {}, h("div", { class: "sheet-title", text: ev ? `${fmtDay(ev.start)}, ${fmtTime(ev.start)}` : "Evento" }), ev ? h("div", { class: "muted", text: `${fmtDur(ev.duration)} · ${fmtAgo(ev.start)}` }) : null),
          h("button", { class: "round flat", "aria-label": "Cerrar", onclick: close }, icon("close"))
        ),
        video,
        h("div", { class: "row" },
          h("div", { class: "row tight" },
            h("button", { class: "btn small", "aria-label": "Más antiguo", disabled: !older, onclick: () => go(1) }, icon("prev")),
            h("button", { class: "btn small", "aria-label": "Más reciente", disabled: !newer, onclick: () => go(-1) }, icon("next"))
          ),
          h("div", { class: "row tight" },
            h("a", { class: "btn small", href: `/api/events/${id}/clip?download=1` }, icon("download"), "Descargar"),
            h("button", { class: "btn small danger", onclick: async () => {
              if (!(await confirmDialog("Borrar evento", "Se eliminará la grabación del servidor. No se puede deshacer.", "Borrar", true))) return;
              try {
                await api(`/api/events/${id}`, { method: "DELETE" });
                state.events = state.events.filter((e) => e.id !== id);
                toast("Evento borrado");
                close();
                if (state.tab === "events") showMain();
              } catch (e) { toast(e.message, "bad"); }
            } }, icon("trash"), "Borrar")
          )
        )
      )
    );
  }
  draw();
  document.body.append(modal);
}

// ---------- settings ----------

async function pushState() {
  if (!("serviceWorker" in navigator) || !("PushManager" in window)) return "unsupported";
  const reg = await navigator.serviceWorker.getRegistration();
  const sub = reg ? await reg.pushManager.getSubscription() : null;
  if (sub) return "on";
  return typeof Notification !== "undefined" && Notification.permission === "denied" ? "denied" : "off";
}

async function renderSettings(body) {
  const [settings, devices, status, pushKey, push] = await Promise.all([
    api("/api/settings"), api("/api/auth/devices"), api("/api/status"), api("/api/push/key"), pushState(),
  ]);
  state.rects = settings.ignore_rects.map((r) => r.slice());

  const standalone = window.matchMedia("(display-mode: standalone)").matches || navigator.standalone;
  const ios = /iPhone|iPad/.test(navigator.userAgent);

  let dirty = false;
  const save = h("button", { class: "btn primary block", text: "Guardar cambios", disabled: true });
  const touch = () => { dirty = true; save.disabled = false; };

  const sens = h("input", { type: "range", min: 1, max: 10, value: settings.sensitivity, "aria-label": "Sensibilidad" });
  const sensLabel = h("span", { class: "muted", text: sensText(settings.sensitivity) });
  sens.addEventListener("input", () => { sensLabel.textContent = sensText(Number(sens.value)); touch(); });

  const retention = h("select", { class: "select", "aria-label": "Conservar eventos" }, [3, 7, 14, 30, 60].map((d) => {
    const o = h("option", { value: d, text: `${d} días` });
    if (d === settings.retention_days) o.selected = true;
    return o;
  }));
  retention.addEventListener("change", touch);

  const zone = zoneEditor(touch);
  save.addEventListener("click", async () => {
    save.disabled = true;
    try {
      await api("/api/settings", { method: "POST", body: { sensitivity: Number(sens.value), ignore_rects: state.rects, retention_days: Number(retention.value) } });
      dirty = false;
      toast("Cambios guardados", "ok");
    } catch (e) { toast(e.message, "bad"); save.disabled = false; }
  });

  const testBtn = h("button", { class: "btn small", text: "Enviar prueba", onclick: async () => {
    try {
      const r = await api("/api/push/test", { method: "POST" });
      toast(r.sent ? `Aviso enviado (${r.sent} canal${r.sent > 1 ? "es" : ""})` : "No hay ningún canal activo", r.sent ? "ok" : "bad");
    } catch (e) { toast(e.message, "bad"); }
  } });

  const webStatus = h("div", { class: "muted" });
  const webBtn = h("button", { class: "btn small" });
  const applyPush = (st) => {
    const canInstall = ios && !standalone;
    webStatus.textContent = st === "on" ? "Activadas en este dispositivo"
      : st === "denied" ? "Permiso denegado en los ajustes del sistema"
      : canInstall ? "En iPhone solo funciona desde la app instalada en la pantalla de inicio"
      : st === "unsupported" ? "Este navegador no admite avisos push"
      : "Desactivadas en este dispositivo";
    webBtn.hidden = st === "unsupported" || st === "denied" || canInstall;
    webBtn.textContent = st === "on" ? "Desactivar" : "Activar";
    webBtn.onclick = async () => {
      try {
        if (st === "on") { await disablePush(); st = "off"; toast("Avisos desactivados"); }
        else { await enablePush(pushKey.key); st = "on"; toast("Avisos activados", "ok"); }
      } catch (e) { toast(e.message, "bad"); }
      applyPush(st);
    };
  };
  applyPush(push);

  const cam = status.camera;
  const pct = Math.min(100, Math.round((status.storage.bytes / status.storage.cap_bytes) * 100));

  body.append(
    pageTitle("Ajustes"),
    h("h2", { text: "Avisos" }),
    h("div", { class: "card stack" },
      h("div", { class: "row" },
        h("div", { class: "row tight grow" }, icon("bell", "dim"), h("div", {}, h("div", { text: "Notifich" }), h("div", { class: "muted", text: status.notifich ? "Llegan a la app Notifich del móvil" : "No configurado en el servidor" }))),
        status.notifich ? chip("Activo", "ok") : chip("Inactivo", "bad")
      ),
      h("hr"),
      h("div", { class: "row" },
        h("div", { class: "row tight grow" }, icon("phone", "dim"), h("div", {}, h("div", { text: "Notificaciones web" }), webStatus)),
        webBtn
      ),
      ios && !standalone ? h("div", { class: "note" }, "Para recibirlas aquí: Safari, botón Compartir, Añadir a pantalla de inicio, y abre la app desde ese icono.") : null,
      h("div", { class: "row end" }, testBtn)
    ),
    h("h2", { text: "Detección" }),
    h("div", { class: "card stack" },
      h("div", { class: "row" }, h("span", { text: "Sensibilidad" }), sensLabel), sens,
      h("hr"),
      h("div", { class: "row" }, h("span", { text: "Conservar eventos" }), retention),
      h("hr"),
      h("div", {}, h("div", { text: "Zonas ignoradas" }), h("div", { class: "muted", text: "Arrastra sobre la imagen para marcar donde no quieres detectar (ventana, ventilador)." })),
      zone, save
    ),
    h("h2", { text: "Sistema" }),
    h("div", { class: "card kv" },
      kv("Cámara", cam.standby ? "Apagada" : cam.online ? "Conectada" : "Sin señal"),
      kv("Señal wifi", cam.rssi ? `${cam.rssi} dBm` : "-"),
      kv("Firmware", cam.fw || "-"),
      kv("Cámara encendida", fmtUptime(cam.uptime_s)),
      kv("Presencia", status.presence.configured ? (status.presence.home ? "En casa" : "Fuera de casa") : "Sin configurar"),
      h("div", { class: "kv-storage" },
        h("div", { class: "row" }, h("span", { text: "Almacenamiento" }), h("span", { class: "muted", text: `${fmtBytes(status.storage.bytes)} de ${fmtBytes(status.storage.cap_bytes)}` })),
        h("div", { class: "bar" }, h("div", { class: "fill" + (pct > 85 ? " warn" : "") })),
        h("div", { class: "muted bar-note", text: `${status.storage.events} eventos guardados` })
      )
    ),
    h("h2", { text: "Seguridad" }),
    devicesCard(devices),
    h("div", { class: "card" }, h("button", { class: "btn danger block", text: "Cerrar sesión", onclick: async () => {
      try { await api("/api/auth/logout", { method: "POST" }); } catch (_) {}
      showAuth();
    } }))
  );
  const fill = body.querySelector(".bar .fill");
  if (fill) fill.style.width = pct + "%";
  const leave = (e) => { if (dirty) { e.preventDefault(); e.returnValue = ""; } };
  window.addEventListener("beforeunload", leave);
  const prevStop = state.stop;
  state.stop = () => { window.removeEventListener("beforeunload", leave); if (prevStop) prevStop(); };
}

function sensText(v) {
  return `${v} de 10 · ${v <= 3 ? "baja" : v <= 7 ? "media" : "alta"}`;
}

function kv(label, value) {
  return h("div", { class: "row kv-row" }, h("span", { class: "muted", text: label }), h("span", { text: value }));
}

function zoneEditor(onChange) {
  const img = h("img", { alt: "Vista para marcar zonas", src: "/api/snapshot?t=" + Date.now(), draggable: "false" });
  const box = h("div", { class: "zone" }, img);
  const draw = () => {
    box.querySelectorAll(".rect").forEach((n) => n.remove());
    state.rects.forEach(([x, y, w, hh]) => {
      const r = h("div", { class: "rect" });
      Object.assign(r.style, { left: x * 100 + "%", top: y * 100 + "%", width: w * 100 + "%", height: hh * 100 + "%" });
      box.append(r);
    });
  };
  let start = null;
  let temp = null;
  const pos = (e) => {
    const b = box.getBoundingClientRect();
    return [Math.min(1, Math.max(0, (e.clientX - b.left) / b.width)), Math.min(1, Math.max(0, (e.clientY - b.top) / b.height))];
  };
  box.addEventListener("pointerdown", (e) => {
    start = pos(e);
    temp = h("div", { class: "rect" });
    box.append(temp);
    box.setPointerCapture(e.pointerId);
  });
  box.addEventListener("pointermove", (e) => {
    if (!start) return;
    const [x, y] = pos(e);
    Object.assign(temp.style, {
      left: Math.min(start[0], x) * 100 + "%", top: Math.min(start[1], y) * 100 + "%",
      width: Math.abs(x - start[0]) * 100 + "%", height: Math.abs(y - start[1]) * 100 + "%",
    });
  });
  box.addEventListener("pointerup", (e) => {
    if (!start) return;
    const [x, y] = pos(e);
    const rect = [Math.min(start[0], x), Math.min(start[1], y), Math.abs(x - start[0]), Math.abs(y - start[1])];
    start = null;
    temp.remove();
    if (rect[2] > 0.02 && rect[3] > 0.02 && state.rects.length < 10) { state.rects.push(rect); onChange(); }
    draw();
  });
  const clear = h("button", { class: "btn small", text: "Borrar zonas", onclick: () => { state.rects = []; draw(); onChange(); } });
  img.addEventListener("load", draw);
  return h("div", { class: "stack" }, box, h("div", { class: "row end" }, clear));
}

async function enablePush(key) {
  if (!("serviceWorker" in navigator) || !("PushManager" in window)) throw new Error("Este navegador no admite avisos push");
  const perm = await Notification.requestPermission();
  if (perm !== "granted") throw new Error("Permiso de notificaciones denegado");
  const reg = await navigator.serviceWorker.ready;
  const sub = (await reg.pushManager.getSubscription()) || (await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: b64uToBuf(key) }));
  await api("/api/push/subscribe", { method: "POST", body: { subscription: sub.toJSON() } });
}

async function disablePush() {
  const reg = await navigator.serviceWorker.getRegistration();
  const sub = reg ? await reg.pushManager.getSubscription() : null;
  if (!sub) return;
  await api("/api/push/unsubscribe", { method: "POST", body: { endpoint: sub.endpoint } });
  await sub.unsubscribe();
}

function devicesCard(devices) {
  const codeBox = h("div");
  return h("div", { class: "card stack" },
    h("div", { class: "muted caps", text: "Passkeys" }),
    h("div", { class: "list" }, devices.credentials.map((c) =>
      h("div", {}, h("div", { class: "row tight grow" }, icon("key", "dim"), h("span", { text: c.name || "Passkey" })),
        h("button", { class: "btn small danger", text: "Quitar", onclick: async () => {
          if (!(await confirmDialog("Quitar passkey", "Ese dispositivo dejará de poder entrar.", "Quitar", true))) return;
          try { await api(`/api/auth/credentials/${encodeURIComponent(c.id)}`, { method: "DELETE" }); showMain(); } catch (e) { toast(e.message, "bad"); }
        } }))
    )),
    h("div", { class: "muted caps", text: "Sesiones abiertas" }),
    h("div", { class: "list" }, devices.sessions.map((s) => {
      const current = s.id === devices.current_session;
      return h("div", {}, h("div", { class: "row tight grow" }, icon("phone", "dim"), h("span", { text: deviceLabel(s.user_agent) }), current ? chip("Este", "ok") : null),
        current ? null : h("button", { class: "btn small danger", text: "Cerrar", onclick: async () => {
          try { await api(`/api/auth/sessions/${s.id}`, { method: "DELETE" }); showMain(); } catch (e) { toast(e.message, "bad"); }
        } }));
    })),
    h("button", { class: "btn block", text: "Añadir otro dispositivo", onclick: async () => {
      try {
        const r = await api("/api/auth/enroll-code", { method: "POST" });
        codeBox.replaceChildren(h("div", { class: "code", text: r.code }), h("div", { class: "muted center-text", text: "Válido 10 minutos. Introdúcelo en el dispositivo nuevo." }));
      } catch (e) { toast(e.message, "bad"); }
    } }),
    codeBox
  );
}

if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});

showAuth().catch((e) => {
  app.replaceChildren(h("div", { class: "empty" }, h("p", { text: e.message })));
});

"use strict";

const app = document.getElementById("app");
const state = { tab: "live", timers: [], rects: [] };

const ICONS = {
  live: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/></svg>',
  events: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18M8 4v5"/></svg>',
  settings: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0"/><circle cx="16" cy="6" r="2"/><circle cx="10" cy="12" r="2"/><circle cx="18" cy="18" r="2"/></svg>',
};

function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v === undefined || v === null || v === false) continue;
    if (k === "class") el.className = v;
    else if (k === "text") el.textContent = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "value" || k === "checked" || k === "disabled") el[k] = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat()) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

function icon(name) {
  const span = document.createElement("span");
  span.innerHTML = ICONS[name];
  return span.firstChild;
}

async function api(path, opts = {}) {
  const init = { method: opts.method || "GET", credentials: "same-origin", headers: {} };
  if (opts.body !== undefined) {
    init.body = JSON.stringify(opts.body);
    init.headers["Content-Type"] = "application/json";
  }
  const res = await fetch(path, init);
  if (res.status === 401 && !path.startsWith("/api/auth/")) {
    clearTimers();
    await showAuth();
    throw new Error("login required");
  }
  if (!res.ok) {
    let detail = res.statusText;
    try { detail = (await res.json()).detail || detail; } catch (_) {}
    throw new Error(detail);
  }
  return res.status === 204 ? null : res.json();
}

function clearTimers() {
  state.timers.forEach(clearInterval);
  state.timers = [];
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
  const y = new Date(); y.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return "Hoy";
  if (d.toDateString() === y.toDateString()) return "Ayer";
  return d.toLocaleDateString("es-ES", { weekday: "long", day: "numeric", month: "long" });
}
function fmtDur(s) {
  if (s === null || s === undefined) return "grabando";
  return s < 60 ? `${Math.round(s)} s` : `${Math.floor(s / 60)} min ${Math.round(s % 60)} s`;
}

// ---------- authentication ----------

async function registerPasskey(code, name) {
  const options = await api("/api/auth/register/options", { method: "POST", body: { code } });
  options.challenge = b64uToBuf(options.challenge);
  options.user.id = b64uToBuf(options.user.id);
  (options.excludeCredentials || []).forEach((c) => (c.id = b64uToBuf(c.id)));
  const cred = await navigator.credentials.create({ publicKey: options });
  await api("/api/auth/register/verify", {
    method: "POST",
    body: {
      code,
      name,
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

async function showAuth() {
  clearTimers();
  const st = await api("/api/auth/state");
  if (st.logged_in) return showMain();

  const err = h("div", { class: "err" });
  const fail = (e) => (err.textContent = e.message || String(e));

  if (!window.PublicKeyCredential) {
    app.replaceChildren(h("div", { class: "center" }, h("h1", { text: "Habitación" }), h("p", { text: "Este navegador no admite passkeys." })));
    return;
  }

  const codeInput = h("input", { type: "text", placeholder: "Código de alta", autocomplete: "off", autocapitalize: "off" });
  const enrollBox = h(
    "div",
    { class: "stack" },
    h("p", { class: "muted", text: "Introduce el codigo que genera el servidor (camctl enroll) o otro dispositivo ya registrado." }),
    codeInput,
    h("button", {
      class: "btn primary block",
      text: "Registrar este dispositivo",
      onclick: async () => {
        try { await registerPasskey(codeInput.value.trim(), navigator.platform || "dispositivo"); await showMain(); } catch (e) { fail(e); }
      },
    })
  );

  const login = h("button", {
    class: "btn primary block",
    text: "Entrar",
    onclick: async () => {
      try { await loginPasskey(); await showMain(); } catch (e) { fail(e); }
    },
  });

  app.replaceChildren(
    h("div", { class: "center" }, h("h1", { text: "Habitación" }), h("p", { text: st.enrolled ? "Acceso con passkey" : "Primer acceso" })),
    h("div", { class: "card stack" }, st.enrolled ? login : null, st.enrolled ? h("div", { class: "muted", text: "Si es un dispositivo nuevo:" }) : null, enrollBox, err)
  );
}

// ---------- shell ----------

async function showMain() {
  clearTimers();
  const tabs = [["live", "En vivo"], ["events", "Eventos"], ["settings", "Ajustes"]];
  const body = h("div");
  const nav = h(
    "nav",
    { class: "tabs" },
    h("div", {}, tabs.map(([id, label]) =>
      h("button", {
        class: state.tab === id ? "on" : "",
        onclick: () => { state.tab = id; showMain(); },
      }, icon(id), label)
    ))
  );
  app.replaceChildren(body, nav);
  try {
    if (state.tab === "live") await renderLive(body);
    else if (state.tab === "events") await renderEvents(body);
    else await renderSettings(body);
  } catch (e) {
    if (e.message !== "login required") body.append(h("div", { class: "center" }, h("p", { text: e.message })));
  }
  const m = location.hash.match(/event=(\d+)/);
  if (m) {
    history.replaceState(null, "", location.pathname);
    openEvent(Number(m[1]));
  }
}

function chip(text, tone) {
  return h("span", { class: "chip" }, h("span", { class: "dot " + (tone || "") }), text);
}

const REASONS = {
  privacy: "Privacidad activa",
  snoozed: "En pausa",
  manual: "Modo manual",
  home: "Estás en casa",
  away: "Fuera de casa",
  "auto-without-device": "Auto sin dispositivo",
};

// ---------- live ----------

async function renderLive(body) {
  const img = h("img", { alt: "Imagen en directo" });
  const overlay = h("div", { class: "overlay", text: "Conectando..." });
  const viewer = h("div", { class: "viewer" }, img, overlay);
  const chips = h("div", { class: "chips" });
  const controls = h("div");
  body.append(h("div", { class: "top" }, h("h1", { text: "Habitación" })), viewer, chips, controls);

  let streaming = false;
  const startStream = () => {
    if (streaming) return;
    streaming = true;
    img.src = "/api/live?t=" + Date.now();
  };
  img.addEventListener("load", () => (overlay.hidden = true));
  img.addEventListener("error", () => {
    streaming = false;
    overlay.hidden = false;
    overlay.textContent = "Sin imagen";
  });

  async function refresh() {
    const s = await api("/api/status");
    chips.replaceChildren(
      chip(s.armed ? "Armado" : "Desarmado", s.armed ? "ok" : "warn"),
      chip(REASONS[s.reason] || s.reason),
      chip(s.camera.standby ? "Cámara apagada" : s.camera.online ? "Cámara conectada" : "Cámara sin señal", s.camera.standby ? "warn" : s.camera.online ? "ok" : "bad"),
      s.recording ? chip("Grabando", "bad") : s.motion ? chip("Movimiento", "warn") : null,
      s.camera.rssi ? chip(`${s.camera.rssi} dBm`) : null
    );
    if (s.camera.standby) {
      overlay.hidden = false;
      overlay.textContent = "Cámara apagada por privacidad";
      img.removeAttribute("src");
      streaming = false;
    } else {
      startStream();
    }
    controls.replaceChildren(controlsCard(s, refresh));
  }
  await refresh();
  every(4000, () => refresh().catch(() => {}));
  every(8000, () => { if (!streaming && !viewer.querySelector("img[src]")) startStream(); });
}

function controlsCard(s, refresh) {
  const modes = [["auto", "Auto"], ["armed", "Armado"], ["disarmed", "Desarmado"]];
  const seg = h("div", { class: "seg" }, modes.map(([id, label]) =>
    h("button", { class: s.mode === id ? "on" : "", text: label, onclick: async () => { await api("/api/mode", { method: "POST", body: { mode: id } }); refresh(); } })
  ));
  const snoozed = s.snooze_until * 1000 > Date.now();
  const snoozeRow = h("div", { class: "row" },
    h("span", { class: "muted", text: snoozed ? "En pausa hasta " + fmtTime(s.snooze_until) : "Pausar avisos" }),
    h("div", { class: "row" },
      snoozed
        ? h("button", { class: "btn", text: "Reanudar", onclick: async () => { await api("/api/snooze", { method: "POST", body: { minutes: 0 } }); refresh(); } })
        : [[60, "1 h"], [480, "8 h"]].map(([m, l]) => h("button", { class: "btn", text: l, onclick: async () => { await api("/api/snooze", { method: "POST", body: { minutes: m } }); refresh(); } }))
    )
  );
  const privacy = h("div", { class: "row" },
    h("div", {}, h("div", { text: "Privacidad" }), h("div", { class: "muted", text: "Apaga el sensor de la cámara" })),
    h("input", { type: "checkbox", class: "switch", checked: s.privacy, onchange: async (e) => {
      try { await api("/api/privacy", { method: "POST", body: { on: e.target.checked } }); } catch (err) { alert(err.message); }
      refresh();
    } })
  );
  const hint = !s.presence.configured && s.mode === "auto" ? h("div", { class: "muted", text: "Modo auto sin dispositivo configurado: no se graba hasta que lo armes." }) : null;
  return h("div", { class: "card stack" }, seg, hint, snoozeRow, privacy);
}

// ---------- events ----------

async function renderEvents(body) {
  body.append(h("div", { class: "top" }, h("h1", { text: "Eventos" })));
  const { events } = await api("/api/events?limit=100");
  if (!events.length) {
    body.append(h("div", { class: "center" }, h("p", { text: "Sin eventos todavía." })));
    return;
  }
  let lastDay = "";
  for (const ev of events) {
    const day = fmtDay(ev.start);
    if (day !== lastDay) { body.append(h("div", { class: "day", text: day })); lastDay = day; }
    body.append(h("button", { class: "event", onclick: () => openEvent(ev.id) },
      ev.status === "done" ? h("img", { src: `/api/events/${ev.id}/thumb`, alt: "", loading: "lazy" }) : h("div"),
      h("div", { class: "meta" }, h("strong", { text: fmtTime(ev.start) }), h("span", { class: "muted", text: ev.status === "failed" ? "Error al guardar" : fmtDur(ev.duration) }))
    ));
  }
}

async function openEvent(id) {
  const close = () => modal.remove();
  const video = h("video", { controls: true, playsinline: true, autoplay: true, src: `/api/events/${id}/clip`, poster: `/api/events/${id}/thumb` });
  const modal = h("div", { class: "modal", onclick: (e) => { if (e.target === modal) close(); } },
    h("div", { class: "box stack" }, video,
      h("div", { class: "row" },
        h("button", { class: "btn danger", text: "Borrar", onclick: async () => {
          if (!confirm("¿Borrar este evento?")) return;
          await api(`/api/events/${id}`, { method: "DELETE" }); close(); showMain();
        } }),
        h("button", { class: "btn", text: "Cerrar", onclick: close })
      )
    )
  );
  document.body.append(modal);
}

// ---------- settings ----------

async function renderSettings(body) {
  const [settings, devices, status, pushKey] = await Promise.all([
    api("/api/settings"), api("/api/auth/devices"), api("/api/status"), api("/api/push/key"),
  ]);
  state.rects = settings.ignore_rects.map((r) => r.slice());

  const sens = h("input", { type: "range", min: 1, max: 10, value: settings.sensitivity });
  const sensLabel = h("span", { class: "muted", text: String(settings.sensitivity) });
  sens.addEventListener("input", () => (sensLabel.textContent = sens.value));

  const retention = h("select", { class: "btn" }, [3, 7, 14, 30, 60].map((d) => {
    const o = h("option", { value: d, text: `${d} días` });
    if (d === settings.retention_days) o.selected = true;
    return o;
  }));

  const msg = h("div", { class: "muted" });
  const zone = zoneEditor();
  const save = h("button", { class: "btn primary block", text: "Guardar", onclick: async () => {
    try {
      await api("/api/settings", { method: "POST", body: { sensitivity: Number(sens.value), ignore_rects: state.rects, retention_days: Number(retention.value) } });
      msg.textContent = "Guardado";
    } catch (e) { msg.textContent = e.message; }
  } });

  const standalone = window.matchMedia("(display-mode: standalone)").matches || navigator.standalone;
  const pushMsg = h("div", { class: "muted", text: `${status.push_subscriptions} dispositivo(s) suscritos` });

  body.append(
    h("div", { class: "top" }, h("h1", { text: "Ajustes" })),
    h("h2", { text: "Detección" }),
    h("div", { class: "card stack" },
      h("div", { class: "row" }, h("span", { text: "Sensibilidad" }), sensLabel), sens,
      h("div", { class: "row" }, h("span", { text: "Conservar eventos" }), retention),
      h("div", {}, h("div", { text: "Zonas ignoradas" }), h("div", { class: "muted", text: "Arrastra sobre la imagen para marcar donde no quieres detectar (ventana, ventilador)." })),
      zone, save, msg
    ),
    h("h2", { text: "Notificaciones" }),
    h("div", { class: "card stack" },
      standalone ? null : h("div", { class: "muted", text: "En iPhone hay que añadir esta página a la pantalla de inicio para recibir avisos." }),
      pushMsg,
      h("div", { class: "row" },
        h("button", { class: "btn primary", text: "Activar aqui", onclick: async () => { try { await enablePush(pushKey.key); pushMsg.textContent = "Activado en este dispositivo"; } catch (e) { pushMsg.textContent = e.message; } } }),
        h("button", { class: "btn", text: "Probar", onclick: async () => { const r = await api("/api/push/test", { method: "POST" }); pushMsg.textContent = `Enviado a ${r.sent}`; } })
      )
    ),
    h("h2", { text: "Dispositivos" }),
    devicesCard(devices),
    h("div", { class: "card" }, h("button", { class: "btn danger block", text: "Cerrar sesión", onclick: async () => { await api("/api/auth/logout", { method: "POST" }); showAuth(); } }))
  );
}

function zoneEditor() {
  const img = h("img", { alt: "", src: "/api/snapshot?t=" + Date.now(), draggable: "false" });
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
    if (rect[2] > 0.02 && rect[3] > 0.02 && state.rects.length < 10) state.rects.push(rect);
    draw();
  });
  const clear = h("button", { class: "btn", text: "Borrar zonas", onclick: () => { state.rects = []; draw(); } });
  img.addEventListener("load", draw);
  return h("div", { class: "stack" }, box, clear);
}

async function enablePush(key) {
  if (!("serviceWorker" in navigator) || !("PushManager" in window)) throw new Error("Este navegador no admite avisos push");
  const reg = await navigator.serviceWorker.register("/sw.js");
  const perm = await Notification.requestPermission();
  if (perm !== "granted") throw new Error("Permiso de notificaciones denegado");
  await navigator.serviceWorker.ready;
  const sub = (await reg.pushManager.getSubscription()) || (await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: b64uToBuf(key) }));
  await api("/api/push/subscribe", { method: "POST", body: { subscription: sub.toJSON() } });
}

function devicesCard(devices) {
  const codeBox = h("div");
  return h("div", { class: "card stack" },
    h("div", { class: "muted", text: "Passkeys" }),
    h("div", { class: "list" }, devices.credentials.map((c) =>
      h("div", {}, h("span", { text: c.name }),
        h("button", { class: "btn danger", text: "Quitar", onclick: async () => {
          if (!confirm("¿Quitar esta passkey?")) return;
          try { await api(`/api/auth/credentials/${encodeURIComponent(c.id)}`, { method: "DELETE" }); showMain(); } catch (e) { alert(e.message); }
        } }))
    )),
    h("div", { class: "muted", text: "Sesiones abiertas" }),
    h("div", { class: "list" }, devices.sessions.map((s) =>
      h("div", {}, h("span", { text: (s.id === devices.current_session ? "Esta - " : "") + (s.user_agent || "desconocido") }),
        s.id === devices.current_session ? null : h("button", { class: "btn danger", text: "Cerrar", onclick: async () => { await api(`/api/auth/sessions/${s.id}`, { method: "DELETE" }); showMain(); } }))
    )),
    h("button", { class: "btn block", text: "Añadir otro dispositivo", onclick: async () => {
      const r = await api("/api/auth/enroll-code", { method: "POST" });
      codeBox.replaceChildren(h("div", { class: "code", text: r.code }), h("div", { class: "muted", text: "Válido 10 minutos. Introdúcelo en el dispositivo nuevo." }));
    } }),
    codeBox
  );
}

showAuth().catch((e) => {
  app.replaceChildren(h("div", { class: "center" }, h("p", { text: e.message })));
});

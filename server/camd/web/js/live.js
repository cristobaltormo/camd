import { api, busy, choiceDialog, chip, fmtAgo, fmtDay, fmtTime, h, icon, prefs, saveToDevice, spinner, toast } from "./lib.js";

const REASONS = {
  privacy: "Privacidad activa: la cámara está apagada",
  snoozed: "Avisos en pausa",
  manual: "Modo manual",
  home: "Estás en casa: no se graba",
  away: "Fuera de casa: vigilando",
  "auto-without-device": "Sin dispositivo de presencia: no se graba",
};
const CLIP_OPTIONS = [[15, "15 s"], [30, "30 s"], [60, "1 min"], [120, "2 min"], [300, "5 min"]];
const POLL_MS = 4000;
const FAST_POLL_MS = 1000;
const WAKE_WINDOW_MS = 25000;

const clock = (s) => `${Math.floor(s / 60)}:${String(Math.max(0, Math.floor(s % 60))).padStart(2, "0")}`;

export function mount(el, navigate) {
  let destroyed = false;
  let pending = 0;
  let wakeUntil = 0;
  let status = null;
  let wasClipping = false;
  const timers = new Set();
  const later = (fn, ms) => { const t = setTimeout(() => { timers.delete(t); if (!destroyed) fn(); }, ms); timers.add(t); return t; };

  // ---------- stream ----------
  const img = h("img", { alt: "Imagen en directo", draggable: "false" });
  const overlay = h("div", { class: "overlay" });
  const liveBadge = h("span", { class: "badge live", hidden: true }, h("span", { class: "pulse" }), "EN VIVO");
  const recBadge = h("span", { class: "badge rec", hidden: true }, h("span", { class: "pulse" }), "GRABANDO");
  const fullBtn = h("button", { class: "round", "aria-label": "Pantalla completa", title: "Pantalla completa (F)" }, icon("expand"));
  const viewer = h("div", { class: "viewer" }, img, overlay, h("div", { class: "badges" }, liveBadge, recBadge), h("div", { class: "viewer-actions" }, fullBtn));

  let phase = "idle";
  let failures = 0;
  let retryTimer = null;
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

  function stopStream() {
    phase = "idle";
    clearInterval(watcher);
    clearTimeout(retryTimer);
    retryTimer = null;
    img.removeAttribute("src");
  }

  function startStream() {
    if (phase !== "idle" || destroyed || document.hidden) return;
    if (!status || status.camera.standby || !status.camera.online) return;
    phase = "connecting";
    showOverlay("wait", failures ? "Reconectando..." : "Conectando...");
    img.src = "/api/live?t=" + Date.now();
    clearInterval(watcher);
    watcher = setInterval(() => {
      if (img.naturalWidth > 0) {
        phase = "playing";
        failures = 0;
        overlay.hidden = true;
        liveBadge.hidden = false;
        clearInterval(watcher);
      }
    }, 250);
  }

  img.addEventListener("error", () => {
    if (phase === "idle" || destroyed) return;
    stopStream();
    failures += 1;
    showOverlay("signal", "Sin imagen", ["Reintentar", () => { failures = 0; startStream(); }]);
    retryTimer = setTimeout(() => { retryTimer = null; startStream(); }, Math.min(15000, 1000 * 2 ** Math.min(failures, 4)));
  });

  const onVisible = () => {
    if (document.hidden) stopStream();
    else { failures = 0; refresh().catch(() => {}); }
  };
  document.addEventListener("visibilitychange", onVisible);

  // ---------- full screen ----------
  const setFull = (on) => {
    viewer.classList.toggle("full", on);
    document.body.classList.toggle("noscroll", on);
    fullBtn.replaceChildren(icon(on ? "shrink" : "expand"));
  };
  const toggleFull = () => {
    const on = !viewer.classList.contains("full");
    setFull(on);
    if (on && viewer.requestFullscreen) viewer.requestFullscreen().catch(() => {});
    if (!on && document.fullscreenElement) document.exitFullscreen().catch(() => {});
  };
  const onFsChange = () => { if (!document.fullscreenElement && viewer.classList.contains("full")) setFull(false); };
  fullBtn.addEventListener("click", toggleFull);
  document.addEventListener("fullscreenchange", onFsChange);

  // ---------- photo ----------
  const photoBtn = h("button", { class: "btn action" }, icon("snap"), h("span", { text: "Foto" }));

  async function photoToWeb() {
    const shot = await api("/api/photos", { method: "POST" });
    toast("Foto guardada en Fotos", "ok", ["Ver", () => navigate(`/photos/${shot.id}`)]);
  }

  async function photoToDevice() {
    const res = await fetch("/api/snapshot", { credentials: "same-origin" });
    if (!res.ok) throw new Error("La cámara no está enviando imagen");
    const blob = await res.blob();
    const done = await saveToDevice(blob, `habitacion-${Date.now()}.jpg`);
    if (done !== "cancelled") toast(done === "shared" ? "Foto lista para guardar" : "Foto descargada", "ok");
  }

  photoBtn.addEventListener("click", () => busy(photoBtn, async () => {
    try {
      let dest = prefs.photoDest;
      if (dest === "ask") {
        const pick = await choiceDialog("Guardar foto", "Elige dónde se guarda esta foto.", [
          ["web", "En la web", "Queda en la galería de Fotos de la aplicación", "cloud"],
          ["device", "En este dispositivo", "Se descarga o se comparte desde aquí", "phone"],
        ], true);
        if (!pick) return;
        dest = pick.value;
        if (pick.remember) prefs.photoDest = dest;
      }
      await (dest === "web" ? photoToWeb() : photoToDevice());
    } catch (e) {
      toast(e.message, "bad");
    }
  }));

  // ---------- clip ----------
  const clipSelect = h("select", { class: "select action-select", "aria-label": "Duración del clip" },
    CLIP_OPTIONS.map(([v, text]) => h("option", { value: v, text, selected: v === prefs.clipSeconds })));
  clipSelect.addEventListener("change", () => { prefs.clipSeconds = Number(clipSelect.value); });
  const clipBtn = h("button", { class: "btn action" });
  let clipTicker = null;

  function drawClip() {
    const clip = status && status.clip;
    const active = !!(clip && clip.active);
    clipBtn.classList.toggle("recording", active);
    clipSelect.hidden = active;
    clearInterval(clipTicker);
    if (!active) {
      clipBtn.replaceChildren(icon("record"), h("span", { text: "Clip" }));
      clipBtn.title = "Grabar un clip y guardarlo";
      return;
    }
    const paint = () => {
      const left = Math.max(0, clip.until - Date.now() / 1000);
      clipBtn.replaceChildren(icon("stop"), h("span", { text: `Parar · ${clock(left)}` }));
    };
    paint();
    clipTicker = setInterval(paint, 500);
    clipBtn.title = "Parar y guardar el clip";
  }

  clipBtn.addEventListener("click", () => busy(clipBtn, async () => {
    try {
      if (status && status.clip.active) {
        await api("/api/clips/stop", { method: "POST" });
      } else {
        await api("/api/clips/start", { method: "POST", body: { seconds: Number(clipSelect.value) } });
      }
    } catch (e) {
      toast(e.message, "bad");
    }
    await refresh().catch(() => {});
  }));

  // ---------- controls (built once, updated in place) ----------
  const hero = h("div", { class: "card hero" });
  const modeButtons = [["auto", "Auto"], ["armed", "Armado"], ["disarmed", "Desarmado"]].map(([id, label]) =>
    h("button", { text: label, "data-mode": id, onclick: () => act(() => api("/api/mode", { method: "POST", body: { mode: id } })) }));
  const seg = h("div", { class: "seg", role: "group", "aria-label": "Modo" }, modeButtons);
  const hint = h("div", { class: "muted", hidden: true, text: "Modo auto sin dispositivo configurado: no se graba hasta que lo armes." });
  const snoozeText = h("div", { class: "muted" });
  const snoozeButtons = h("div", { class: "row tight" });
  const privacySwitch = h("input", { type: "checkbox", class: "switch", "aria-label": "Privacidad" });
  const controls = h("div", { class: "card stack" },
    seg, hint, h("hr"),
    h("div", { class: "row" }, h("div", {}, h("div", { text: "Pausar avisos" }), snoozeText), snoozeButtons),
    h("hr"),
    h("div", { class: "row" }, h("div", {}, h("div", { text: "Privacidad" }), h("div", { class: "muted", text: "Apaga el sensor de la cámara" })), privacySwitch)
  );
  const last = h("div");

  async function act(fn) {
    pending += 1;
    try { await fn(); } catch (e) { toast(e.message, "bad"); }
    pending -= 1;
    await refresh().catch(() => {});
  }

  privacySwitch.addEventListener("change", async () => {
    const want = privacySwitch.checked;
    privacySwitch.disabled = true;
    pending += 1;
    try {
      await api("/api/privacy", { method: "POST", body: { on: want } });
      if (want) { stopStream(); } else { wakeUntil = Date.now() + WAKE_WINDOW_MS; failures = 0; showOverlay("wait", "Encendiendo la cámara..."); }
    } catch (e) {
      privacySwitch.checked = !want;
      toast(e.message, "bad");
    }
    pending -= 1;
    privacySwitch.disabled = false;
    await refresh().catch(() => {});
  });

  function applyControls(s) {
    modeButtons.forEach((b) => {
      const on = b.dataset.mode === s.mode;
      b.classList.toggle("on", on);
      b.setAttribute("aria-pressed", String(on));
    });
    hint.hidden = !(!s.presence.configured && s.mode === "auto");
    const snoozed = s.snooze_until * 1000 > Date.now();
    snoozeText.textContent = snoozed ? "En pausa hasta las " + fmtTime(s.snooze_until) : "No se grabará ni se avisará";
    snoozeButtons.replaceChildren(...(snoozed
      ? [h("button", { class: "btn small", text: "Reanudar", onclick: () => act(() => api("/api/snooze", { method: "POST", body: { minutes: 0 } })) })]
      : [[60, "1 h"], [480, "8 h"]].map(([m, l]) => h("button", { class: "btn small", text: l, onclick: () => act(() => api("/api/snooze", { method: "POST", body: { minutes: m } })) }))));
    if (!privacySwitch.disabled) privacySwitch.checked = s.privacy;
  }

  // ---------- status loop ----------
  async function refresh() {
    const s = await api("/api/status");
    if (destroyed) return;
    status = s;
    recBadge.hidden = !s.recording;
    renderHero(hero, s);
    last.replaceChildren(lastEvent(s, navigate));
    if (!pending) applyControls(s);

    const clipping = s.clip.active;
    if (wasClipping && !clipping) toast("Clip guardado en Eventos", "ok", ["Ver", () => navigate("/events")]);
    wasClipping = clipping;
    drawClip();
    clipBtn.disabled = !clipping && (s.camera.standby || !s.camera.online);
    photoBtn.disabled = s.camera.standby || !s.camera.online;

    if (s.camera.standby) {
      stopStream();
      showOverlay("standby", "Cámara apagada por privacidad", ["Encender", () => { privacySwitch.checked = false; privacySwitch.dispatchEvent(new Event("change")); }]);
    } else if (!s.camera.online) {
      stopStream();
      showOverlay(Date.now() < wakeUntil ? "wait" : "signal", Date.now() < wakeUntil ? "Encendiendo la cámara..." : "Cámara sin señal");
    } else {
      wakeUntil = 0;
      startStream();
    }
  }

  function schedule() {
    if (destroyed) return;
    later(async () => {
      try { await refresh(); } catch (_) {}
      schedule();
    }, Date.now() < wakeUntil ? FAST_POLL_MS : POLL_MS);
  }

  // ---------- keyboard ----------
  const onKey = (e) => {
    if (e.target.matches("input, select, textarea") || e.metaKey || e.ctrlKey || e.altKey || document.querySelector(".modal")) return;
    if (e.key === "f" || e.key === "F") toggleFull();
    else if (e.key === "s" || e.key === "S") photoBtn.click();
    else if (e.key === "Escape" && viewer.classList.contains("full")) setFull(false);
  };
  document.addEventListener("keydown", onKey);

  el.append(
    h("header", { class: "page-head" }, h("h1", { text: "En vivo" })),
    h("div", { class: "live-grid" },
      h("div", { class: "stage" }, h("div", { class: "stage-inner" }, viewer, h("div", { class: "actions" }, photoBtn, h("div", { class: "clip-control" }, clipBtn, clipSelect)))),
      h("aside", { class: "side" }, hero, controls, last))
  );

  return {
    ready: refresh().then(schedule),
    update() {},
    destroy() {
      destroyed = true;
      timers.forEach(clearTimeout);
      clearInterval(clipTicker);
      stopStream();
      document.removeEventListener("visibilitychange", onVisible);
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("fullscreenchange", onFsChange);
      if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
      document.body.classList.remove("noscroll");
    },
  };
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

function lastEvent(s, navigate) {
  const ev = s.last_event;
  if (!ev) return h("div");
  return h("button", { class: "card link-card", onclick: () => navigate(`/events/${ev.id}`) },
    icon("film"),
    h("div", { class: "grow" }, h("div", { text: "Último movimiento" }), h("div", { class: "muted", text: `${fmtDay(ev.start_ts)}, ${fmtTime(ev.start_ts)} · ${fmtAgo(ev.start_ts)}` })),
    icon("next", "dim")
  );
}

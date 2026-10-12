import { api, chip, fmtAgo, fmtDay, fmtTime, h, icon, spinner, toast } from "./lib.js";

const REASONS = {
  privacy: "Privacidad activa: la cámara está apagada",
  snoozed: "Avisos en pausa",
  manual: "Modo manual",
  home: "Estás en casa: no se graba",
  away: "Fuera de casa: vigilando",
  "auto-without-device": "Sin dispositivo de presencia: no se graba",
};

export function mount(el, navigate) {
  const img = h("img", { alt: "Imagen en directo", draggable: "false" });
  const overlay = h("div", { class: "overlay" });
  const liveBadge = h("span", { class: "badge live", hidden: true }, h("span", { class: "pulse" }), "EN VIVO");
  const recBadge = h("span", { class: "badge rec", hidden: true }, h("span", { class: "pulse" }), "GRABANDO");
  const fullBtn = h("button", { class: "round", "aria-label": "Pantalla completa", title: "Pantalla completa (F)" }, icon("expand"));
  const snapBtn = h("a", { class: "round", "aria-label": "Guardar foto", title: "Guardar foto (S)", href: "/api/snapshot?download=1" }, icon("snap"));
  const viewer = h("div", { class: "viewer" }, img, overlay, h("div", { class: "badges" }, liveBadge, recBadge), h("div", { class: "viewer-actions" }, snapBtn, fullBtn));

  const hero = h("div", { class: "card hero" });
  const controls = h("div");
  const last = h("div");
  el.append(
    h("header", { class: "page-head" }, h("h1", { text: "En vivo" })),
    h("div", { class: "live-grid" }, h("div", { class: "stage" }, viewer), h("aside", { class: "side" }, hero, controls, last))
  );

  let streaming = false;
  let standby = false;
  let watcher = null;
  let retry = null;
  let failures = 0;
  let destroyed = false;

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

  const onKey = (e) => {
    if (e.target.matches("input, select, textarea") || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === "f" || e.key === "F") toggleFull();
    else if (e.key === "s" || e.key === "S") snapBtn.click();
    else if (e.key === "Escape" && viewer.classList.contains("full")) setFull(false);
  };
  document.addEventListener("keydown", onKey);

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
    clearTimeout(retry);
    retry = null;
    img.removeAttribute("src");
  };

  const startStream = () => {
    if (streaming || standby || destroyed) return;
    streaming = true;
    showOverlay("wait", failures ? "Reconectando..." : "Conectando...");
    img.src = "/api/live?t=" + Date.now();
    clearInterval(watcher);
    watcher = setInterval(() => {
      if (img.naturalWidth > 0) {
        failures = 0;
        overlay.hidden = true;
        liveBadge.hidden = false;
        clearInterval(watcher);
      }
    }, 250);
  };

  img.addEventListener("error", () => {
    if (!streaming || destroyed) return;
    stopStream();
    failures += 1;
    const wait = Math.min(15000, 1000 * 2 ** Math.min(failures, 4));
    showOverlay("signal", "Sin imagen", ["Reintentar", () => { failures = 0; stopStream(); startStream(); }]);
    retry = setTimeout(() => { retry = null; startStream(); }, wait);
  });

  const onVisible = () => {
    if (document.hidden) stopStream();
    else if (!standby) startStream();
  };
  document.addEventListener("visibilitychange", onVisible);

  async function refresh() {
    const s = await api("/api/status");
    standby = s.camera.standby;
    recBadge.hidden = !s.recording;
    renderHero(hero, s);
    controls.replaceChildren(controlsCard(s, refresh));
    last.replaceChildren(lastEvent(s, navigate));
    if (standby) {
      stopStream();
      showOverlay("standby", "Cámara apagada por privacidad", ["Encender", async () => {
        try { await api("/api/privacy", { method: "POST", body: { on: false } }); } catch (e) { toast(e.message, "bad"); }
        refresh();
      }]);
    } else if (!streaming && !document.hidden && !retry) {
      startStream();
    }
  }

  const timer = setInterval(() => refresh().catch(() => {}), 4000);

  return {
    ready: refresh(),
    update() {},
    destroy() {
      destroyed = true;
      clearInterval(timer);
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

function lastEvent(s, navigate) {
  const ev = s.last_event;
  if (!ev) return h("div");
  return h("button", { class: "card link-card", onclick: () => navigate(`/events/${ev.id}`) },
    icon("film"),
    h("div", { class: "grow" }, h("div", { text: "Último movimiento" }), h("div", { class: "muted", text: `${fmtDay(ev.start_ts)}, ${fmtTime(ev.start_ts)} · ${fmtAgo(ev.start_ts)}` })),
    icon("next", "dim")
  );
}

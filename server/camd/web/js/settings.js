import { api, b64uToBuf, chip, confirmDialog, deviceLabel, fmtBytes, fmtUptime, h, icon, isIOS, isStandalone, prefs, toast } from "./lib.js";

const SECTIONS = [
  ["alerts", "Avisos", "bell"],
  ["detection", "Detección", "sliders"],
  ["recording", "Grabación", "film"],
  ["captures", "Capturas", "snap"],
  ["presence", "Presencia", "presence"],
  ["camera", "Cámara", "camera"],
  ["appearance", "Apariencia", "palette"],
  ["app", "Aplicación", "app"],
  ["system", "Sistema", "disk"],
  ["security", "Seguridad", "key"],
];

const MIN = (n) => n * 60;
const OPTIONS = {
  push_cooldown_s: [[30, "30 segundos"], [60, "1 minuto"], [120, "2 minutos"], [300, "5 minutos"], [600, "10 minutos"], [1800, "30 minutos"]],
  pre_roll_s: [[0, "Sin pregrabación"], [2, "2 segundos"], [3, "3 segundos"], [5, "5 segundos"], [10, "10 segundos"]],
  post_roll_s: [[5, "5 segundos"], [10, "10 segundos"], [20, "20 segundos"], [30, "30 segundos"], [60, "1 minuto"]],
  max_event_s: [[30, "30 segundos"], [60, "1 minuto"], [120, "2 minutos"], [180, "3 minutos"], [300, "5 minutos"]],
  retention_days: [[3, "3 días"], [7, "7 días"], [14, "14 días"], [30, "30 días"], [60, "60 días"], [90, "90 días"]],
  max_disk_gb: [[1, "1 GB"], [2, "2 GB"], [4, "4 GB"], [6, "6 GB"], [8, "8 GB"], [10, "10 GB"]],
  presence_grace_s: [[MIN(1), "1 minuto"], [MIN(2), "2 minutos"], [MIN(5), "5 minutos"], [MIN(10), "10 minutos"], [MIN(15), "15 minutos"], [MIN(30), "30 minutos"]],
  camera_quality: [[0, "Predeterminada"], [8, "Máxima"], [12, "Alta"], [18, "Media"], [28, "Baja (menos datos)"]],
};

const sensText = (v) => `${v} de 10 · ${v <= 3 ? "baja" : v <= 7 ? "media" : "alta"}`;
const signed = (v) => (v > 0 ? `+${v}` : String(v));

export function mount(el, navigate, ctx) {
  const st = { server: null, draft: null, status: null, devices: null, pushKey: null, push: "off", section: SECTIONS.some(([id]) => id === ctx.sub) ? ctx.sub : "alerts", saving: false };
  const content = h("div", { class: "settings-body" });
  const nav = h("nav", { class: "subnav", "aria-label": "Secciones de ajustes" });
  const bar = h("div", { class: "savebar", hidden: true, role: "region", "aria-label": "Cambios sin guardar" });
  el.append(
    h("header", { class: "page-head" }, h("h1", { text: "Ajustes" })),
    h("div", { class: "settings-layout" }, nav, content),
    bar
  );

  const dirtyKeys = () => Object.keys(st.draft).filter((k) => JSON.stringify(st.draft[k]) !== JSON.stringify(st.server[k]));

  function refreshBar() {
    const keys = dirtyKeys();
    bar.hidden = !keys.length;
    if (!keys.length) return;
    bar.replaceChildren(
      h("span", { text: `${keys.length} ${keys.length === 1 ? "cambio sin guardar" : "cambios sin guardar"}` }),
      h("div", { class: "row tight" },
        h("button", { class: "btn small", text: "Descartar", disabled: st.saving, onclick: () => { st.draft = structuredClone(st.server); drawSection(); refreshBar(); } }),
        h("button", { class: "btn small primary", text: st.saving ? "Guardando..." : "Guardar", disabled: st.saving, onclick: save })
      )
    );
  }

  async function save() {
    const keys = dirtyKeys();
    const patch = Object.fromEntries(keys.map((k) => [k, st.draft[k]]));
    st.saving = true;
    refreshBar();
    try {
      await api("/api/settings", { method: "POST", body: patch });
      st.server = await api("/api/settings");
      st.draft = structuredClone(st.server);
      toast("Cambios guardados", "ok");
      if (keys.some((k) => k.startsWith("camera_"))) setTimeout(() => window.dispatchEvent(new Event("cam:camera-changed")), 1500);
    } catch (e) {
      toast(e.message, "bad");
    }
    st.saving = false;
    refreshBar();
  }

  const change = (key, value) => { st.draft[key] = value; refreshBar(); };

  const setting = (label, hint, control, extra) =>
    h("div", { class: "setting" }, h("div", { class: "setting-text" }, h("div", { class: "setting-label", text: label }), hint ? h("div", { class: "muted", text: hint }) : null), h("div", { class: "setting-control" }, control), extra || null);

  const toggle = (key, label, hint) => {
    const input = h("input", { type: "checkbox", class: "switch", "aria-label": label, checked: !!st.draft[key], onchange: (e) => { change(key, e.target.checked); onToggle(key); } });
    return setting(label, hint, input);
  };

  const select = (key, label, hint) => {
    const opts = OPTIONS[key];
    const current = st.draft[key];
    const list = opts.some(([v]) => v === current) ? opts : [...opts, [current, String(current)]].sort((a, b) => a[0] - b[0]);
    const sel = h("select", { class: "select", "aria-label": label, onchange: (e) => change(key, Number(e.target.value)) },
      list.map(([v, text]) => h("option", { value: v, text, selected: v === current })));
    return setting(label, hint, sel);
  };

  const slider = (key, label, hint, min, max, fmt = String) => {
    const out = h("span", { class: "slider-out", text: fmt(st.draft[key]) });
    const input = h("input", { type: "range", min, max, step: 1, value: st.draft[key], "aria-label": label });
    input.addEventListener("input", () => { out.textContent = fmt(Number(input.value)); change(key, Number(input.value)); });
    return h("div", { class: "setting column" },
      h("div", { class: "row" }, h("div", { class: "setting-text" }, h("div", { class: "setting-label", text: label }), hint ? h("div", { class: "muted", text: hint }) : null), out),
      input);
  };

  const card = (title, ...rows) => h("section", { class: "card panel" }, title ? h("h2", { class: "panel-title", text: title }) : null, ...rows);

  let onToggle = () => {};

  // ---------- sections ----------

  function alertsSection() {
    const quietStart = h("input", { type: "time", class: "time", value: st.draft.quiet_start, "aria-label": "Inicio del silencio", disabled: !st.draft.quiet_enabled, onchange: (e) => change("quiet_start", e.target.value || st.server.quiet_start) });
    const quietEnd = h("input", { type: "time", class: "time", value: st.draft.quiet_end, "aria-label": "Fin del silencio", disabled: !st.draft.quiet_enabled, onchange: (e) => change("quiet_end", e.target.value || st.server.quiet_end) });
    onToggle = (key) => { if (key === "quiet_enabled") { quietStart.disabled = quietEnd.disabled = !st.draft.quiet_enabled; } };

    const webStatus = h("div", { class: "muted" });
    const webBtn = h("button", { class: "btn small" });
    const applyPush = (state) => {
      const needsInstall = isIOS() && !isStandalone();
      webStatus.textContent = state === "on" ? "Activadas en este dispositivo"
        : state === "denied" ? "Permiso denegado en los ajustes del sistema"
        : needsInstall ? "En iPhone solo funcionan desde la app instalada en la pantalla de inicio"
        : state === "unsupported" ? "Este navegador no admite avisos push"
        : "Desactivadas en este dispositivo";
      webBtn.hidden = state === "unsupported" || state === "denied" || needsInstall;
      webBtn.textContent = state === "on" ? "Desactivar" : "Activar";
      webBtn.onclick = async () => {
        try {
          if (state === "on") { await disablePush(); state = "off"; toast("Avisos desactivados"); }
          else { await enablePush(st.pushKey); state = "on"; toast("Avisos activados", "ok"); }
        } catch (e) { toast(e.message, "bad"); }
        st.push = state;
        applyPush(state);
      };
    };
    applyPush(st.push);

    const test = h("button", { class: "btn small", text: "Enviar prueba", onclick: async () => {
      try {
        const r = await api("/api/push/test", { method: "POST" });
        toast(r.sent ? `Aviso enviado (${r.sent} canal${r.sent > 1 ? "es" : ""})` : "No hay ningún canal activo", r.sent ? "ok" : "bad");
      } catch (e) { toast(e.message, "bad"); }
    } });

    return [
      card("Canales",
        h("div", { class: "setting" },
          h("div", { class: "row tight setting-text" }, icon("bell", "dim"), h("div", {}, h("div", { class: "setting-label", text: "Notifich" }), h("div", { class: "muted", text: st.status.notifich ? "Llegan a la app Notifich del móvil" : "No configurado en el servidor" }))),
          st.status.notifich ? chip("Activo", "ok") : chip("Inactivo", "bad")),
        h("div", { class: "setting" },
          h("div", { class: "row tight setting-text" }, icon("monitor", "dim"), h("div", {}, h("div", { class: "setting-label", text: "Notificaciones web" }), webStatus)),
          webBtn),
        h("div", { class: "row end" }, test)),
      card("Qué avisar",
        toggle("notify_motion", "Movimiento", "Aviso cuando se empieza a grabar un evento"),
        toggle("notify_offline", "Cámara sin conexión", "Aviso si no llegan imágenes durante 5 minutos"),
        select("push_cooldown_s", "Tiempo mínimo entre avisos", "Evita avalanchas si hay mucho movimiento")),
      card("Horas de silencio",
        toggle("quiet_enabled", "Silenciar avisos de movimiento", "Se sigue grabando, solo se calla el aviso"),
        setting("Desde", null, quietStart),
        setting("Hasta", null, quietEnd)),
    ];
  }

  function detectionSection() {
    const rects = st.draft.ignore_rects;
    const zone = zoneEditor(rects, (next) => change("ignore_rects", next));
    return [
      card("Sensibilidad",
        slider("sensitivity", "Sensibilidad al movimiento", "Más alta detecta cambios más pequeños, pero da más falsos avisos", 1, 10, sensText)),
      card("Zonas ignoradas",
        h("div", { class: "muted", text: "Arrastra sobre la imagen para marcar donde no quieres detectar (ventana, ventilador, pantalla)." }),
        zone),
    ];
  }

  function recordingSection() {
    return [
      card("Clips",
        select("pre_roll_s", "Pregrabación", "Segundos guardados antes de que empiece el movimiento"),
        select("post_roll_s", "Seguir grabando tras el movimiento", "Cuánto esperar sin movimiento antes de cerrar el clip"),
        select("max_event_s", "Duración máxima de un clip")),
      card("Almacenamiento",
        select("retention_days", "Conservar eventos", "Los más antiguos se borran solos"),
        select("max_disk_gb", "Espacio máximo", "Si se llena, se borran los más antiguos primero"),
        storageBlock()),
    ];
  }

  function capturesSection() {
    const dest = prefs.photoDest;
    const seg = h("div", { class: "seg", role: "group", "aria-label": "Destino de las fotos" }, [["ask", "Preguntar"], ["web", "En la web"], ["device", "En el dispositivo"]].map(([id, label]) =>
      h("button", { class: dest === id ? "on" : "", text: label, "aria-pressed": String(dest === id), onclick: () => { prefs.photoDest = id; drawSection(); } })));
    const clip = h("select", { class: "select", "aria-label": "Duración por defecto del clip", onchange: (e) => { prefs.clipSeconds = Number(e.target.value); } },
      [[15, "15 segundos"], [30, "30 segundos"], [60, "1 minuto"], [120, "2 minutos"], [300, "5 minutos"]].map(([v, t]) => h("option", { value: v, text: t, selected: v === prefs.clipSeconds })));
    const u = st.status.storage;
    return [
      card("Fotos",
        h("div", { class: "setting column" }, h("div", { class: "setting-text" }, h("div", { class: "setting-label", text: "Dónde se guardan" }), h("div", { class: "muted", text: "En la web quedan en la galería de Fotos del servidor; en el dispositivo se descargan o se comparten desde el móvil." })), seg),
        h("div", { class: "setting" }, h("div", { class: "setting-text" }, h("div", { class: "setting-label", text: "Guardadas en la web" }), h("div", { class: "muted", text: `${u.photos} fotos · ${fmtBytes(u.photo_bytes)}` })), h("a", { class: "btn small", href: "#/photos", text: "Abrir" }))),
      card("Clips",
        setting("Duración por defecto", "Los clips manuales se guardan en Eventos y no se borran solos", clip)),
      card(null, h("div", { class: "muted", text: "Estas preferencias se guardan solo en este dispositivo." })),
    ];
  }

  function storageBlock() {
    const used = st.status.storage;
    const pct = Math.min(100, Math.round((used.bytes / (st.draft.max_disk_gb * 1024 ** 3)) * 100));
    const fill = h("div", { class: "fill" + (pct > 85 ? " warn" : "") });
    fill.style.width = pct + "%";
    return h("div", { class: "kv-storage" },
      h("div", { class: "row" }, h("span", { class: "muted", text: "Eventos" }), h("span", { text: `${fmtBytes(used.bytes)} · ${used.events}` })),
      h("div", { class: "bar" }, fill),
      h("div", { class: "row spaced" }, h("span", { class: "muted", text: "Fotos" }), h("span", { text: `${fmtBytes(used.photo_bytes)} · ${used.photos}` })));
  }

  function presenceSection() {
    const p = st.status.presence;
    return [
      card("Estado",
        h("div", { class: "setting" },
          h("div", { class: "row tight setting-text" }, icon("presence", "dim"), h("div", {}, h("div", { class: "setting-label", text: p.configured ? (p.home ? "Estás en casa" : "Fuera de casa") : "Sin dispositivo configurado" }), h("div", { class: "muted", text: p.configured ? "Detectado por la red wifi de casa" : "Define presence_devices en la configuración del servidor" }))),
          chip(p.configured ? (p.home ? "En casa" : "Fuera") : "Sin configurar", p.configured ? (p.home ? "ok" : "warn") : "bad"))),
      card("Modo automático",
        select("presence_grace_s", "Tiempo de gracia", "Los iPhone duermen la wifi: se espera este tiempo antes de darte por fuera de casa"),
        h("div", { class: "note", text: "En modo Auto la cámara graba cuando no estás. El modo manual (Armado o Desarmado) se cambia desde En vivo." })),
    ];
  }

  function cameraSection() {
    const img = h("img", { alt: "Vista previa de la cámara", src: "/api/snapshot?t=" + Date.now(), draggable: "false" });
    const refresh = () => { img.src = "/api/snapshot?t=" + Date.now(); };
    window.addEventListener("cam:camera-changed", refresh);
    st.cleanupSection = () => window.removeEventListener("cam:camera-changed", refresh);
    const cam = st.status.camera;
    return [
      card("Vista previa",
        h("div", { class: "preview" }, img),
        h("div", { class: "row end" }, h("button", { class: "btn small", onclick: refresh }, icon("refresh"), "Actualizar")),
        h("div", { class: "muted", text: "Los cambios se aplican a la cámara al guardar y se recuerdan si se reinicia." })),
      card("Imagen",
        toggle("camera_vflip", "Voltear verticalmente", "Si la cámara está montada del revés"),
        toggle("camera_hmirror", "Espejo horizontal"),
        slider("camera_brightness", "Brillo", null, -2, 2, signed),
        slider("camera_contrast", "Contraste", null, -2, 2, signed),
        select("camera_quality", "Calidad de imagen", "Más calidad pesa más: con wifi flojo puede ir a saltos")),
      card("Dispositivo",
        kv("Estado", cam.standby ? "Apagada (privacidad)" : cam.online ? "Conectada" : "Sin señal"),
        kv("Señal wifi", cam.rssi ? `${cam.rssi} dBm` : "-"),
        kv("Firmware", cam.fw || "-"),
        kv("Encendida desde hace", fmtUptime(cam.uptime_s))),
    ];
  }

  function appearanceSection() {
    const theme = localStorage.getItem("cam.theme") || "auto";
    const accent = localStorage.getItem("cam.accent") || "blue";
    const seg = h("div", { class: "seg", role: "group", "aria-label": "Tema" }, [["auto", "Automático"], ["dark", "Oscuro"], ["light", "Claro"]].map(([id, label]) =>
      h("button", { class: theme === id ? "on" : "", text: label, "aria-pressed": String(theme === id), onclick: () => { window.camTheme.set("cam.theme", id); drawSection(); } })));
    const names = { blue: "Azul", green: "Verde", violet: "Violeta", orange: "Naranja", pink: "Rosa" };
    const swatches = h("div", { class: "swatches", role: "group", "aria-label": "Color de acento" }, Object.entries(window.camTheme.accents).map(([id, color]) => {
      const b = h("button", { class: "swatch" + (accent === id ? " on" : ""), "aria-label": names[id], "aria-pressed": String(accent === id), title: names[id], onclick: () => { window.camTheme.set("cam.accent", id); drawSection(); } });
      b.style.background = color;
      return b;
    }));
    return [card("Tema", seg, setting("Color de acento", null, swatches)), card(null, h("div", { class: "muted", text: "La apariencia se guarda solo en este dispositivo." }))];
  }

  function appSection() {
    const standalone = isStandalone();
    const install = ctx.install;
    let installRow;
    if (standalone) installRow = setting("Aplicación instalada", "Estás usando la versión instalada", chip("Instalada", "ok"));
    else if (install.available()) installRow = setting("Instalar la aplicación", "Se abre en su propia ventana, sin barra del navegador", h("button", { class: "btn small primary", text: "Instalar", onclick: async () => { await install.run(); drawSection(); } }));
    else if (isIOS()) installRow = setting("Instalar en iPhone", "Safari: botón Compartir, luego Añadir a pantalla de inicio. Abre la app desde ese icono.", null);
    else installRow = setting("Instalar la aplicación", "Desde el menú del navegador: Instalar aplicación o Añadir a pantalla de inicio.", null);

    const version = h("span", { class: "muted", text: `Versión ${st.status.version || "-"}` });
    const check = h("button", { class: "btn small", text: "Buscar actualizaciones", onclick: async () => {
      const reg = await navigator.serviceWorker?.getRegistration();
      if (!reg) return toast("Sin service worker en este navegador", "bad");
      await reg.update().catch(() => {});
      toast(reg.waiting || reg.installing ? "Hay una versión nueva, recarga para usarla" : "Ya tienes la última versión");
    } });
    const clear = h("button", { class: "btn small danger", text: "Vaciar caché local", onclick: async () => {
      if (!(await confirmDialog("Vaciar caché local", "Se borran los archivos guardados para uso sin conexión y se recarga la aplicación.", "Vaciar", false))) return;
      for (const key of await caches.keys()) await caches.delete(key);
      location.reload();
    } });

    const keys = [["1", "En vivo"], ["2", "Eventos"], ["3", "Fotos"], ["4", "Ajustes"], ["F", "Pantalla completa (En vivo)"], ["S", "Guardar foto (En vivo)"], ["Esc", "Cerrar"], ["← →", "Evento anterior o siguiente"]];
    return [
      card("Instalación", installRow),
      card("Mantenimiento", setting("Actualizaciones", null, check), setting("Caché local", "Útil si la aplicación se queda con datos viejos", clear), h("div", { class: "row end" }, version)),
      card("Atajos de teclado", h("div", { class: "keys" }, keys.map(([k, d]) => h("div", { class: "row" }, h("span", { text: d }), h("kbd", { text: k }))))),
    ];
  }

  function kv(label, value) {
    return h("div", { class: "row kv-row" }, h("span", { class: "muted", text: label }), h("span", { text: value }));
  }

  function systemSection() {
    const s = st.status;
    const cam = s.camera;
    return [
      card("Servidor",
        kv("Versión", s.version || "-"),
        kv("Presencia", s.presence.configured ? (s.presence.home ? "En casa" : "Fuera de casa") : "Sin configurar"),
        kv("Avisos Notifich", s.notifich ? "Activos" : "Inactivos"),
        kv("Dispositivos web suscritos", String(s.push_subscriptions))),
      card("Cámara",
        kv("Estado", cam.standby ? "Apagada" : cam.online ? "Conectada" : "Sin señal"),
        kv("Señal wifi", cam.rssi ? `${cam.rssi} dBm` : "-"),
        kv("Firmware", cam.fw || "-"),
        kv("Encendida desde hace", fmtUptime(cam.uptime_s))),
      card("Almacenamiento", storageBlock()),
    ];
  }

  function securitySection() {
    const d = st.devices;
    const codeBox = h("div");
    const others = d.sessions.filter((s) => s.id !== d.current_session);
    return [
      card("Passkeys",
        h("div", { class: "list" }, d.credentials.map((c) =>
          h("div", {}, h("div", { class: "row tight grow" }, icon("key", "dim"), h("span", { text: c.name || "Passkey" })),
            h("button", { class: "btn small danger", text: "Quitar", onclick: async () => {
              if (!(await confirmDialog("Quitar passkey", "Ese dispositivo dejará de poder entrar.", "Quitar", true))) return;
              try { await api(`/api/auth/credentials/${encodeURIComponent(c.id)}`, { method: "DELETE" }); await reload(); } catch (e) { toast(e.message, "bad"); }
            } })))),
        h("button", { class: "btn block", text: "Añadir otro dispositivo", onclick: async () => {
          try {
            const r = await api("/api/auth/enroll-code", { method: "POST" });
            codeBox.replaceChildren(h("div", { class: "code", text: r.code }), h("div", { class: "muted center-text", text: "Válido 10 minutos. Introdúcelo en el dispositivo nuevo." }));
          } catch (e) { toast(e.message, "bad"); }
        } }),
        codeBox),
      card("Sesiones abiertas",
        h("div", { class: "list" }, d.sessions.map((s) => {
          const current = s.id === d.current_session;
          return h("div", {}, h("div", { class: "row tight grow" }, icon("monitor", "dim"), h("span", { text: deviceLabel(s.user_agent) }), current ? chip("Este", "ok") : null),
            current ? null : h("button", { class: "btn small danger", text: "Cerrar", onclick: async () => {
              try { await api(`/api/auth/sessions/${s.id}`, { method: "DELETE" }); await reload(); } catch (e) { toast(e.message, "bad"); }
            } }));
        })),
        others.length ? h("button", { class: "btn block danger", text: `Cerrar las otras ${others.length} sesiones`, onclick: async () => {
          if (!(await confirmDialog("Cerrar otras sesiones", "Los demás dispositivos tendrán que volver a entrar con su passkey.", "Cerrar", true))) return;
          try { for (const s of others) await api(`/api/auth/sessions/${s.id}`, { method: "DELETE" }); await reload(); toast("Sesiones cerradas"); } catch (e) { toast(e.message, "bad"); }
        } }) : null),
      card(null, h("button", { class: "btn danger block", text: "Cerrar sesión en este dispositivo", onclick: async () => {
        try { await api("/api/auth/logout", { method: "POST" }); } catch (_) {}
        location.reload();
      } })),
    ];
  }

  const RENDER = { alerts: alertsSection, detection: detectionSection, recording: recordingSection, captures: capturesSection, presence: presenceSection, camera: cameraSection, appearance: appearanceSection, app: appSection, system: systemSection, security: securitySection };

  function drawNav() {
    nav.replaceChildren(...SECTIONS.map(([id, label, ic]) =>
      h("a", { href: `#/settings/${id}`, class: "subnav-item" + (st.section === id ? " on" : ""), "aria-current": st.section === id ? "page" : null }, icon(ic), h("span", { text: label }))));
    nav.querySelector(".on")?.scrollIntoView({ block: "nearest", inline: "center" });
  }

  function drawSection() {
    if (st.cleanupSection) { st.cleanupSection(); st.cleanupSection = null; }
    onToggle = () => {};
    drawNav();
    const [, label] = SECTIONS.find(([id]) => id === st.section);
    content.replaceChildren(h("h2", { class: "section-title", text: label }), ...RENDER[st.section]());
  }

  async function reload() {
    [st.status, st.devices] = await Promise.all([api("/api/status"), api("/api/auth/devices")]);
    drawSection();
  }

  const leave = (e) => { if (dirtyKeys().length) { e.preventDefault(); e.returnValue = ""; } };
  window.addEventListener("beforeunload", leave);

  return {
    ready: Promise.all([api("/api/settings"), api("/api/auth/devices"), api("/api/status"), api("/api/push/key"), pushState()]).then(([settings, devices, status, key, push]) => {
      st.server = settings;
      st.draft = structuredClone(settings);
      st.devices = devices;
      st.status = status;
      st.pushKey = key.key;
      st.push = push;
      drawSection();
    }),
    update(sub) {
      const next = SECTIONS.some(([id]) => id === sub) ? sub : "alerts";
      if (next !== st.section) {
        st.section = next;
        if (st.draft) drawSection();
      } else if (st.draft) drawNav();
    },
    destroy() {
      window.removeEventListener("beforeunload", leave);
      if (st.cleanupSection) st.cleanupSection();
    },
  };
}

async function pushState() {
  if (!("serviceWorker" in navigator) || !("PushManager" in window)) return "unsupported";
  const reg = await navigator.serviceWorker.getRegistration();
  const sub = reg ? await reg.pushManager.getSubscription() : null;
  if (sub) return "on";
  return typeof Notification !== "undefined" && Notification.permission === "denied" ? "denied" : "off";
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

function zoneEditor(initial, onChange) {
  let rects = initial.map((r) => r.slice());
  const img = h("img", { alt: "Vista para marcar zonas", src: "/api/snapshot?t=" + Date.now(), draggable: "false" });
  const box = h("div", { class: "zone" }, img);
  const draw = () => {
    box.querySelectorAll(".rect").forEach((n) => n.remove());
    rects.forEach(([x, y, w, hh], i) => {
      const r = h("div", { class: "rect" });
      Object.assign(r.style, { left: x * 100 + "%", top: y * 100 + "%", width: w * 100 + "%", height: hh * 100 + "%" });
      r.append(h("span", { class: "rect-n", text: String(i + 1) }));
      box.append(r);
    });
    counter.textContent = rects.length ? `${rects.length} ${rects.length === 1 ? "zona" : "zonas"}` : "Sin zonas";
  };
  const counter = h("span", { class: "muted" });
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
    if (rect[2] > 0.02 && rect[3] > 0.02 && rects.length < 10) { rects.push(rect); onChange(rects.map((r) => r.slice())); }
    draw();
  });
  const undo = h("button", { class: "btn small", text: "Deshacer", onclick: () => { rects.pop(); onChange(rects.map((r) => r.slice())); draw(); } });
  const clear = h("button", { class: "btn small", text: "Borrar todas", onclick: () => { rects = []; onChange([]); draw(); } });
  img.addEventListener("load", draw);
  draw();
  return h("div", { class: "stack" }, box, h("div", { class: "row" }, counter, h("div", { class: "row tight" }, undo, clear)));
}

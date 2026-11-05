import { api, b64uToBuf, cameraState, chip, confirmDialog, deviceLabel, fmtBytes, fmtUptime, h, icon, isIOS, isStandalone, labelName, prefs, toast } from "./lib.js";
import { LANGUAGES, count, preference, setLanguage, t } from "./text.js";

const SECTIONS = [
  ["alerts", "Alerts", "bell"],
  ["detection", "Detection", "scan"],
  ["recording", "Recording", "film"],
  ["captures", "Captures", "snap"],
  ["presence", "Presence", "presence"],
  ["camera", "Camera", "camera"],
  ["appearance", "Appearance", "palette"],
  ["app", "App", "app"],
  ["system", "System", "disk"],
  ["security", "Security", "key"],
];

const COMMON_LABELS = ["person", "cat", "dog", "bird", "car", "bicycle"];
const DETECTION_MODES = [
  ["off", "Off", "Motion only: every change of the image is recorded and alerted."],
  ["alert", "Alert on objects", "Every motion is recorded, but you are only alerted when a person or animal is seen. Clips without an object are shown apart and deleted early."],
  ["strict", "Only objects", "Only clips with a person or animal are kept. Shadows, light changes and insects are discarded without being stored."],
];

const seconds = (n) => (n < 60 ? count(n, "{n} second", "{n} seconds") : count(n / 60, "{n} minute", "{n} minutes"));
const days = (n) => count(n, "{n} day", "{n} days");
const hours = (n) => (n >= 24 ? days(n / 24) : count(n, "{n} hour", "{n} hours"));

const OPTIONS = {
  push_cooldown_s: [[30, 60, 120, 300, 600, 1800], seconds],
  pre_roll_s: [[0, 2, 3, 5, 10], (v) => (v ? seconds(v) : t("No pre-roll"))],
  post_roll_s: [[5, 10, 20, 30, 60], seconds],
  max_event_s: [[30, 60, 120, 180, 300], seconds],
  retention_days: [[3, 7, 14, 30, 60, 90], days],
  unconfirmed_keep_h: [[1, 6, 12, 24, 48, 168], hours],
  max_disk_gb: [[1, 2, 4, 6, 8, 10, 20, 50], (v) => `${v} GB`],
  presence_grace_s: [[60, 120, 300, 600, 900, 1800], seconds],
  detect_confidence: [[0.3, 0.4, 0.5, 0.6, 0.7, 0.8], (v) => `${Math.round(v * 100)}%`],
  camera_quality: [[0, 8, 12, 18, 28], (v) => ({ 0: t("Default"), 8: t("Maximum"), 12: t("High"), 18: t("Medium"), 28: t("Low (less data)") })[v]],
  camera_framesize: [[0, 5, 8, 9, 10], (v) => ({ 0: t("Default"), 5: "320 × 240", 8: "640 × 480", 9: "800 × 600", 10: "1024 × 768" })[v]],
};

const sensitivityText = (v) => t("{n} of 10 · {level}", { n: v, level: v <= 3 ? t("low") : v <= 7 ? t("medium") : t("high") });
const signed = (v) => (v > 0 ? `+${v}` : String(v));

export function mount(el, navigate, ctx) {
  const st = { server: null, draft: null, status: null, devices: null, pushKey: null, push: "off", section: SECTIONS.some(([id]) => id === ctx.sub) ? ctx.sub : "alerts", saving: false };
  const content = h("div", { class: "settings-body" });
  const nav = h("nav", { class: "subnav", "aria-label": t("Settings sections") });
  const bar = h("div", { class: "savebar", hidden: true, role: "region", "aria-label": t("Unsaved changes") });
  el.append(h("header", { class: "page-head" }, h("h1", { text: t("Settings") })), h("div", { class: "settings-layout" }, nav, content), bar);

  const dirtyKeys = () => Object.keys(st.draft).filter((k) => JSON.stringify(st.draft[k]) !== JSON.stringify(st.server[k]));

  function refreshBar() {
    const keys = dirtyKeys();
    bar.hidden = !keys.length;
    if (!keys.length) return;
    bar.replaceChildren(
      h("span", { text: count(keys.length, "{n} unsaved change", "{n} unsaved changes") }),
      h("div", { class: "row tight" },
        h("button", { class: "btn small", text: t("Discard"), disabled: st.saving, onclick: () => { st.draft = structuredClone(st.server); drawSection(); refreshBar(); } }),
        h("button", { class: "btn small primary", text: st.saving ? t("Saving...") : t("Save"), disabled: st.saving, onclick: save })
      )
    );
  }

  async function save() {
    const keys = dirtyKeys();
    st.saving = true;
    refreshBar();
    try {
      await api("/api/settings", { method: "POST", body: Object.fromEntries(keys.map((k) => [k, st.draft[k]])) });
      st.server = await api("/api/settings");
      st.draft = structuredClone(st.server);
      st.status = await api("/api/status");
      toast(t("Changes saved"), "ok");
      if (keys.some((k) => k.startsWith("camera_"))) setTimeout(() => window.dispatchEvent(new Event("cam:camera-changed")), 1500);
      if (keys.includes("detection") || keys.includes("detect_labels")) drawSection();
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
    const [values, format] = OPTIONS[key];
    const current = st.draft[key];
    const all = values.includes(current) ? values : [...values, current].sort((a, b) => a - b);
    const control = h("select", { class: "select", "aria-label": label, onchange: (e) => change(key, Number(e.target.value)) },
      all.map((v) => h("option", { value: v, text: format(v) ?? String(v), selected: v === current })));
    return setting(label, hint, control);
  };

  const choice = (key, label, hint, options) => {
    const control = h("select", { class: "select", "aria-label": label, onchange: (e) => change(key, e.target.value) },
      options.map(([value, text]) => h("option", { value, text, selected: value === st.draft[key] })));
    return setting(label, hint, control);
  };

  const slider = (key, label, hint, min, max, format = String) => {
    const out = h("span", { class: "slider-out", text: format(st.draft[key]) });
    const input = h("input", { type: "range", min, max, step: 1, value: st.draft[key], "aria-label": label });
    input.addEventListener("input", () => { out.textContent = format(Number(input.value)); change(key, Number(input.value)); });
    return h("div", { class: "setting column" },
      h("div", { class: "row" }, h("div", { class: "setting-text" }, h("div", { class: "setting-label", text: label }), hint ? h("div", { class: "muted", text: hint }) : null), out),
      input);
  };

  const card = (title, ...rows) => h("section", { class: "card panel" }, title ? h("h2", { class: "panel-title", text: title }) : null, ...rows);
  const kv = (label, value) => h("div", { class: "row kv-row" }, h("span", { class: "muted", text: label }), h("span", { text: value }));

  let onToggle = () => {};

  function alertsSection() {
    const quietStart = h("input", { type: "time", class: "time", value: st.draft.quiet_start, "aria-label": t("Quiet hours start"), disabled: !st.draft.quiet_enabled, onchange: (e) => change("quiet_start", e.target.value || st.server.quiet_start) });
    const quietEnd = h("input", { type: "time", class: "time", value: st.draft.quiet_end, "aria-label": t("Quiet hours end"), disabled: !st.draft.quiet_enabled, onchange: (e) => change("quiet_end", e.target.value || st.server.quiet_end) });
    onToggle = (key) => { if (key === "quiet_enabled") quietStart.disabled = quietEnd.disabled = !st.draft.quiet_enabled; };

    const webStatus = h("div", { class: "muted" });
    const webBtn = h("button", { class: "btn small" });
    const applyPush = (state) => {
      const needsInstall = isIOS() && !isStandalone();
      webStatus.textContent = state === "on" ? t("Enabled on this device")
        : state === "denied" ? t("Permission denied in the system settings")
        : needsInstall ? t("On iPhone they only work from the app added to the home screen")
        : state === "unsupported" ? t("This browser does not support push alerts")
        : t("Disabled on this device");
      webBtn.hidden = state === "unsupported" || state === "denied" || needsInstall;
      webBtn.textContent = state === "on" ? t("Disable") : t("Enable");
      webBtn.onclick = async () => {
        try {
          if (state === "on") { await disablePush(); state = "off"; toast(t("Alerts disabled")); }
          else { await enablePush(st.pushKey); state = "on"; toast(t("Alerts enabled"), "ok"); }
        } catch (e) { toast(e.message, "bad"); }
        st.push = state;
        applyPush(state);
      };
    };
    applyPush(st.push);

    const test = h("button", { class: "btn small", text: t("Send a test"), onclick: async () => {
      try {
        const r = await api("/api/push/test", { method: "POST" });
        toast(r.sent ? count(r.sent, "Alert sent ({n} channel)", "Alert sent ({n} channels)") : t("No channel is active"), r.sent ? "ok" : "bad");
      } catch (e) { toast(e.message, "bad"); }
    } });

    const external = st.status.channels.map((name) =>
      h("div", { class: "setting" }, h("div", { class: "row tight setting-text" }, icon("bell", "dim"), h("div", {}, h("div", { class: "setting-label", text: name }), h("div", { class: "muted", text: t("Configured on the server") }))), chip(t("Active"), "ok")));

    return [
      card(t("Channels"),
        ...external,
        h("div", { class: "setting" }, h("div", { class: "row tight setting-text" }, icon("monitor", "dim"), h("div", {}, h("div", { class: "setting-label", text: t("Web notifications") }), webStatus)), webBtn),
        h("div", { class: "row end" }, test)),
      card(t("What to alert"),
        toggle("notify_motion", t("Detections"), st.draft.detection === "off" ? t("Alert when a clip starts recording") : t("Alert when a person or animal is seen")),
        toggle("notify_offline", t("Camera offline"), t("Alert if no images arrive for 5 minutes")),
        select("push_cooldown_s", t("Minimum time between alerts"), t("Avoids floods when there is a lot of activity")),
        choice("language", t("Language of the alerts"), null, LANGUAGES.filter(([id]) => id !== "auto"))),
      card(t("Quiet hours"),
        toggle("quiet_enabled", t("Silence detection alerts"), t("Recording continues, only the alert is muted")),
        setting(t("From"), null, quietStart),
        setting(t("Until"), null, quietEnd)),
    ];
  }

  function labelGrid() {
    const selected = new Set(st.draft.detect_labels);
    const box = (label) => h("label", { class: "check" },
      h("input", { type: "checkbox", checked: selected.has(label), onchange: (e) => {
        if (e.target.checked) selected.add(label); else selected.delete(label);
        if (!selected.size) { selected.add(label); e.target.checked = true; toast(t("Pick at least one object"), "bad"); return; }
        change("detect_labels", [...selected]);
      } }), h("span", { text: labelName(label) }));
    const others = st.server.available_labels.filter((label) => !COMMON_LABELS.includes(label));
    return h("div", { class: "stack" },
      h("div", { class: "checks" }, COMMON_LABELS.map(box)),
      h("details", { class: "more-labels" }, h("summary", { text: t("More objects") }), h("div", { class: "checks" }, others.map(box))));
  }

  function detectionSection() {
    const detection = st.status.detection;
    const modeButtons = DETECTION_MODES.map(([id, label]) => h("button", { class: st.draft.detection === id ? "on" : "", text: t(label), "aria-pressed": String(st.draft.detection === id), onclick: () => { change("detection", id); drawSection(); } }));
    const description = DETECTION_MODES.find(([id]) => id === st.draft.detection)[2];
    const state = detection.mode === "off" ? chip(t("Off"), "warn")
      : detection.state === "ready" ? chip(t("Ready · {ms} ms per image", { ms: Math.round(detection.inference_ms) || "-" }), "ok")
      : detection.state === "error" ? chip(t("Unavailable"), "bad")
      : chip(t("Loading the model"), "warn");
    const active = st.draft.detection !== "off";
    return [
      card(t("Object detection"),
        h("div", { class: "muted", text: t("A small neural network looks at the frames where something moves and tells people and animals apart from shadows, light changes and insects. It runs on the server, with no cloud.") }),
        h("div", { class: "seg", role: "group", "aria-label": t("Detection mode") }, modeButtons),
        h("div", { class: "note", text: t(description) }),
        h("div", { class: "row" }, h("span", { class: "muted", text: t("Detector") }), state),
        detection.error ? h("div", { class: "err", text: detection.error }) : null),
      active ? card(t("What to look for"),
        labelGrid(),
        select("detect_confidence", t("Minimum confidence"), t("Lower sees more objects but is wrong more often")),
        st.draft.detection === "alert" ? select("unconfirmed_keep_h", t("Keep clips without objects for"), t("They are not alerted and are deleted early")) : null) : null,
      card(t("Motion sensitivity"),
        slider("sensitivity", t("Sensitivity to motion"), t("Higher notices smaller changes but gives more false alarms"), 1, 10, sensitivityText)),
      card(t("Ignored zones"),
        h("div", { class: "muted", text: t("Drag over the image to mark where you do not want to detect (window, fan, screen).") }),
        zoneEditor(st.draft.ignore_rects, (next) => change("ignore_rects", next))),
      st.draft.detection !== "off" ? card(t("Alert zones"),
        h("div", { class: "muted", text: t("Alert only when an object is inside these zones. Leave it empty to alert everywhere.") }),
        zoneEditor(st.draft.alert_rects, (next) => change("alert_rects", next), "alert")) : null,
    ];
  }

  function recordingSection() {
    return [
      card(t("Clips"),
        select("pre_roll_s", t("Pre-roll"), t("Seconds kept from before the motion starts")),
        select("post_roll_s", t("Keep recording after the activity"), t("How long to wait without activity before closing the clip")),
        select("max_event_s", t("Maximum length of a clip"))),
      card(t("Storage"),
        select("retention_days", t("Keep events for"), t("The oldest are deleted automatically")),
        select("max_disk_gb", t("Maximum space"), t("When it fills up, the oldest are deleted first")),
        storageBlock()),
    ];
  }

  function capturesSection() {
    const destination = prefs.photoDest;
    const segment = h("div", { class: "seg", role: "group", "aria-label": t("Where photos go") }, [["ask", t("Ask")], ["web", t("On the server")], ["device", t("On the device")]].map(([id, label]) =>
      h("button", { class: destination === id ? "on" : "", text: label, "aria-pressed": String(destination === id), onclick: () => { prefs.photoDest = id; drawSection(); } })));
    const clip = h("select", { class: "select", "aria-label": t("Default clip length"), onchange: (e) => { prefs.clipSeconds = Number(e.target.value); } },
      [15, 30, 60, 120, 300].map((v) => h("option", { value: v, text: seconds(v), selected: v === prefs.clipSeconds })));
    const usage = st.status.storage;
    return [
      card(t("Photos"),
        h("div", { class: "setting column" }, h("div", { class: "setting-text" }, h("div", { class: "setting-label", text: t("Where they are saved") }), h("div", { class: "muted", text: t("On the server they stay in the Photos gallery; on the device they are downloaded or shared from your phone.") })), segment),
        h("div", { class: "setting" }, h("div", { class: "setting-text" }, h("div", { class: "setting-label", text: t("Saved on the server") }), h("div", { class: "muted", text: `${count(usage.photos, "{n} photo", "{n} photos")} · ${fmtBytes(usage.photo_bytes)}` })), h("a", { class: "btn small", href: "#/photos", text: t("Open") }))),
      card(t("Clips"), setting(t("Default length"), t("Manual clips are saved in Events and never deleted automatically"), clip)),
      card(null, h("div", { class: "muted", text: t("These preferences are stored only on this device.") })),
    ];
  }

  function storageBlock() {
    const used = st.status.storage;
    const percent = Math.min(100, Math.round((used.bytes / (st.draft.max_disk_gb * 1024 ** 3)) * 100));
    const fill = h("div", { class: "fill" + (percent > 85 ? " warn" : "") });
    fill.style.width = percent + "%";
    return h("div", { class: "kv-storage" },
      h("div", { class: "row" }, h("span", { class: "muted", text: t("Events") }), h("span", { text: `${fmtBytes(used.bytes)} · ${used.events}` })),
      h("div", { class: "bar" }, fill),
      h("div", { class: "row spaced" }, h("span", { class: "muted", text: t("Photos") }), h("span", { text: `${fmtBytes(used.photo_bytes)} · ${used.photos}` })));
  }

  function presenceSection() {
    const p = st.status.presence;
    return [
      card(t("Status"),
        h("div", { class: "setting" },
          h("div", { class: "row tight setting-text" }, icon("presence", "dim"), h("div", {},
            h("div", { class: "setting-label", text: p.configured ? (p.home ? t("You are home") : t("You are away")) : t("No device configured") }),
            h("div", { class: "muted", text: p.configured ? t("Detected on the home network") : t("Set presence_devices in the server configuration") }))),
          chip(p.configured ? (p.home ? t("Home") : t("Away")) : t("Not set"), p.configured ? (p.home ? "ok" : "warn") : "bad"))),
      card(t("Automatic mode"),
        select("presence_grace_s", t("Grace period"), t("Phones put wifi to sleep: this much time passes before you count as away")),
        h("div", { class: "note", text: t("In Auto mode the camera records when you are away. The manual modes (Armed or Disarmed) are changed from Live.") })),
    ];
  }

  function cameraSection() {
    const img = h("img", { alt: t("Camera preview"), src: "/api/snapshot?t=" + Date.now(), draggable: "false" });
    const refresh = () => { img.src = "/api/snapshot?t=" + Date.now(); };
    window.addEventListener("cam:camera-changed", refresh);
    st.cleanupSection = () => window.removeEventListener("cam:camera-changed", refresh);
    const cam = st.status.camera;
    const power = h("input", { type: "checkbox", class: "switch", "aria-label": t("Camera in use"), checked: cam.enabled, onchange: async (e) => {
      const on = e.target.checked;
      e.target.disabled = true;
      try {
        await api("/api/settings", { method: "POST", body: { camera_enabled: on } });
        st.server.camera_enabled = on;
        st.draft.camera_enabled = on;
        st.status = await api("/api/status");
        toast(on ? t("Camera enabled: I will look for it as soon as it is plugged in") : t("Camera disabled: nothing is recorded or alerted"), on ? "ok" : undefined);
        refreshBar();
        drawSection();
      } catch (err) {
        e.target.checked = !on;
        e.target.disabled = false;
        toast(err.message, "bad");
      }
    } });
    return [
      card(t("Status"), setting(t("Camera in use"), t("Turn it off before unplugging it: the app stops looking for it, does not alert and uses no resources. When you turn it on, everything comes back."), power)),
      card(t("Preview"),
        h("div", { class: "preview" }, img),
        h("div", { class: "row end" }, h("button", { class: "btn small", onclick: refresh }, icon("refresh"), t("Refresh"))),
        h("div", { class: "muted", text: t("Changes are applied to the camera when you save and are remembered after a restart.") })),
      card(t("Image"),
        toggle("camera_vflip", t("Flip vertically"), t("If the camera is mounted upside down")),
        toggle("camera_hmirror", t("Mirror horizontally")),
        slider("camera_brightness", t("Brightness"), null, -2, 2, signed),
        slider("camera_contrast", t("Contrast"), null, -2, 2, signed),
        select("camera_framesize", t("Resolution"), t("A smaller image uses less wifi and CPU on the server")),
        select("camera_quality", t("Image quality"), t("More quality weighs more: with weak wifi it may stutter"))),
      card(t("Device"), ...deviceRows(cam)),
    ];
  }

  const deviceRows = (cam) => [
    kv(t("Status"), cameraState(cam).text),
    kv(t("Wifi signal"), cam.online && cam.rssi ? `${cam.rssi} dBm` : "-"),
    kv(t("Frame rate"), cam.online && cam.fps ? `${cam.fps} fps` : "-"),
    kv(t("Firmware"), cam.fw || "-"),
    kv(t("Powered on for"), cam.online ? fmtUptime(cam.uptime_s) : "-"),
  ];

  function appearanceSection() {
    const theme = localStorage.getItem("cam.theme") || "auto";
    const accent = localStorage.getItem("cam.accent") || "blue";
    const segment = h("div", { class: "seg", role: "group", "aria-label": t("Theme") }, [["auto", t("Automatic")], ["dark", t("Dark")], ["light", t("Light")]].map(([id, label]) =>
      h("button", { class: theme === id ? "on" : "", text: label, "aria-pressed": String(theme === id), onclick: () => { window.camTheme.set("cam.theme", id); drawSection(); } })));
    const names = { blue: t("Blue"), green: t("Green"), violet: t("Violet"), orange: t("Orange"), pink: t("Pink") };
    const swatches = h("div", { class: "swatches", role: "group", "aria-label": t("Accent color") }, Object.entries(window.camTheme.accents).map(([id, color]) => {
      const button = h("button", { class: "swatch" + (accent === id ? " on" : ""), "aria-label": names[id], "aria-pressed": String(accent === id), title: names[id], onclick: () => { window.camTheme.set("cam.accent", id); drawSection(); } });
      button.style.background = color;
      return button;
    }));
    const language = h("select", { class: "select", "aria-label": t("Language"), onchange: (e) => { setLanguage(e.target.value); location.reload(); } },
      LANGUAGES.map(([id, label]) => h("option", { value: id, text: id === "auto" ? t("Automatic") : label, selected: preference() === id })));
    return [card(t("Theme"), segment, setting(t("Accent color"), null, swatches), setting(t("Language"), t("Of this interface"), language)), card(null, h("div", { class: "muted", text: t("Appearance is stored only on this device.") }))];
  }

  function appSection() {
    const install = ctx.install;
    let installRow;
    if (isStandalone()) installRow = setting(t("App installed"), t("You are using the installed version"), chip(t("Installed"), "ok"));
    else if (install.available()) installRow = setting(t("Install the app"), t("It opens in its own window, without the browser bar"), h("button", { class: "btn small primary", text: t("Install"), onclick: async () => { await install.run(); drawSection(); } }));
    else if (isIOS()) installRow = setting(t("Install on iPhone"), t("Safari: Share button, then Add to Home Screen. Open the app from that icon."), null);
    else installRow = setting(t("Install the app"), t("From the browser menu: Install app or Add to home screen."), null);

    const check = h("button", { class: "btn small", text: t("Check for updates"), onclick: async () => {
      const registration = await navigator.serviceWorker?.getRegistration();
      if (!registration) return toast(t("No service worker in this browser"), "bad");
      await registration.update().catch(() => {});
      toast(registration.waiting || registration.installing ? t("A new version is ready, reload to use it") : t("You already have the latest version"));
    } });
    const clear = h("button", { class: "btn small danger", text: t("Clear local cache"), onclick: async () => {
      if (!(await confirmDialog(t("Clear local cache"), t("The files kept for offline use are deleted and the app reloads."), t("Clear"), false))) return;
      for (const key of await caches.keys()) await caches.delete(key);
      location.reload();
    } });

    const keys = [["1", t("Live")], ["2", t("Events")], ["3", t("Photos")], ["4", t("Settings")], ["F", t("Full screen (Live)")], ["S", t("Save photo (Live)")], ["Esc", t("Close")], ["← →", t("Previous or next event")]];
    return [
      card(t("Installation"), installRow),
      card(t("Maintenance"), setting(t("Updates"), null, check), setting(t("Local cache"), t("Useful if the app is stuck with old data"), clear), h("div", { class: "row end" }, h("span", { class: "muted", text: t("Version {version}", { version: st.status.version || "-" }) }))),
      card(t("Keyboard shortcuts"), h("div", { class: "keys" }, keys.map(([key, text]) => h("div", { class: "row" }, h("span", { text }), h("kbd", { text: key }))))),
    ];
  }

  function systemSection() {
    const s = st.status;
    const detector = s.detection.mode === "off" ? t("Off") : s.detection.state === "ready" ? t("Ready") : s.detection.state === "error" ? t("Unavailable") : t("Loading");
    return [
      card(t("Server"),
        kv(t("Version"), s.version || "-"),
        kv(t("Presence"), s.presence.configured ? (s.presence.home ? t("Home") : t("Away")) : t("Not set")),
        kv(t("Object detector"), detector),
        kv(t("Other alert channels"), s.channels.length ? s.channels.join(", ") : t("None")),
        kv(t("Subscribed web devices"), String(s.push_subscriptions))),
      card(t("Camera"), ...deviceRows(s.camera)),
      card(t("Storage"), storageBlock()),
    ];
  }

  function securitySection() {
    const d = st.devices;
    const codeBox = h("div");
    const others = d.sessions.filter((s) => s.id !== d.current_session);
    const passkeys = d.credentials.length ? [h("div", { class: "list" }, d.credentials.map((c) =>
      h("div", {}, h("div", { class: "row tight grow" }, icon("key", "dim"), h("span", { text: c.name || t("Passkey") })),
        h("button", { class: "btn small danger", text: t("Remove"), onclick: async () => {
          if (!(await confirmDialog(t("Remove passkey"), t("That device will no longer be able to sign in."), t("Remove"), true))) return;
          try { await api(`/api/auth/credentials/${encodeURIComponent(c.id)}`, { method: "DELETE" }); await reload(); } catch (e) { toast(e.message, "bad"); }
        } })))) ] : [h("div", { class: "muted", text: t("No passkeys registered.") })];
    return [
      card(t("Passkeys"), ...passkeys,
        h("button", { class: "btn block", text: t("Add another device"), onclick: async () => {
          try {
            const r = await api("/api/auth/enroll-code", { method: "POST" });
            codeBox.replaceChildren(h("div", { class: "code", text: r.code }), h("div", { class: "muted center-text", text: t("Valid for 10 minutes. Enter it on the new device.") }));
          } catch (e) { toast(e.message, "bad"); }
        } }),
        codeBox),
      card(t("Password"), h("div", { class: "muted", text: d.password ? t("Password sign-in is enabled. Change it on the server with camd-ctl password.") : t("Password sign-in is not set up. Create one on the server with camd-ctl password.") })),
      card(t("Open sessions"),
        h("div", { class: "list" }, d.sessions.map((s) => {
          const current = s.id === d.current_session;
          return h("div", {}, h("div", { class: "row tight grow" }, icon("monitor", "dim"), h("span", { text: deviceLabel(s.user_agent) }), current ? chip(t("This one"), "ok") : null),
            current ? null : h("button", { class: "btn small danger", text: t("Close"), onclick: async () => {
              try { await api(`/api/auth/sessions/${s.id}`, { method: "DELETE" }); await reload(); } catch (e) { toast(e.message, "bad"); }
            } }));
        })),
        others.length ? h("button", { class: "btn block danger", text: count(others.length, "Close the other {n} session", "Close the other {n} sessions"), onclick: async () => {
          if (!(await confirmDialog(t("Close other sessions"), t("The other devices will have to sign in again."), t("Close"), true))) return;
          try { for (const s of others) await api(`/api/auth/sessions/${s.id}`, { method: "DELETE" }); await reload(); toast(t("Sessions closed")); } catch (e) { toast(e.message, "bad"); }
        } }) : null),
      card(null, h("button", { class: "btn danger block", text: t("Sign out on this device"), onclick: async () => {
        try { await api("/api/auth/logout", { method: "POST" }); } catch (_) {}
        location.reload();
      } })),
    ];
  }

  const RENDER = { alerts: alertsSection, detection: detectionSection, recording: recordingSection, captures: capturesSection, presence: presenceSection, camera: cameraSection, appearance: appearanceSection, app: appSection, system: systemSection, security: securitySection };

  function drawNav() {
    nav.replaceChildren(...SECTIONS.map(([id, label, ic]) =>
      h("a", { href: `#/settings/${id}`, class: "subnav-item" + (st.section === id ? " on" : ""), "aria-current": st.section === id ? "page" : null }, icon(ic), h("span", { text: t(label) }))));
    nav.querySelector(".on")?.scrollIntoView({ block: "nearest", inline: "center" });
  }

  function drawSection() {
    if (st.cleanupSection) { st.cleanupSection(); st.cleanupSection = null; }
    onToggle = () => {};
    drawNav();
    const [, label] = SECTIONS.find(([id]) => id === st.section);
    content.replaceChildren(h("h2", { class: "section-title", text: t(label) }), ...RENDER[st.section]());
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
  const registration = await navigator.serviceWorker.getRegistration();
  const subscription = registration ? await registration.pushManager.getSubscription() : null;
  if (subscription) return "on";
  return typeof Notification !== "undefined" && Notification.permission === "denied" ? "denied" : "off";
}

async function enablePush(key) {
  if (!("serviceWorker" in navigator) || !("PushManager" in window)) throw new Error(t("This browser does not support push alerts"));
  if ((await Notification.requestPermission()) !== "granted") throw new Error(t("Notification permission denied"));
  const registration = await navigator.serviceWorker.ready;
  const subscription = (await registration.pushManager.getSubscription()) || (await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: b64uToBuf(key) }));
  await api("/api/push/subscribe", { method: "POST", body: { subscription: subscription.toJSON() } });
}

async function disablePush() {
  const registration = await navigator.serviceWorker.getRegistration();
  const subscription = registration ? await registration.pushManager.getSubscription() : null;
  if (!subscription) return;
  await api("/api/push/unsubscribe", { method: "POST", body: { endpoint: subscription.endpoint } });
  await subscription.unsubscribe();
}

const percent = (value) => value * 100 + "%";

function zoneEditor(initial, onChange, kind = "ignore") {
  let rects = initial.map((r) => r.slice());
  const img = h("img", { alt: t("Preview to mark zones"), src: "/api/snapshot?t=" + Date.now(), draggable: "false" });
  const box = h("div", { class: "zone " + kind }, img);
  const counter = h("span", { class: "muted" });
  let start = null;
  let temp = null;

  const place = (el, x, y, w, hh) => Object.assign(el.style, { left: percent(x), top: percent(y), width: percent(w), height: percent(hh) });
  const emit = () => onChange(rects.map((r) => r.slice()));

  const draw = () => {
    box.querySelectorAll(".rect").forEach((n) => n.remove());
    rects.forEach(([x, y, w, hh], i) => {
      const rect = h("div", { class: "rect" }, h("span", { class: "rect-n", text: String(i + 1) }));
      place(rect, x, y, w, hh);
      box.append(rect);
    });
    counter.textContent = rects.length ? count(rects.length, "{n} zone", "{n} zones") : t("No zones");
  };

  const position = (e) => {
    const bounds = box.getBoundingClientRect();
    return [Math.min(1, Math.max(0, (e.clientX - bounds.left) / bounds.width)), Math.min(1, Math.max(0, (e.clientY - bounds.top) / bounds.height))];
  };
  box.addEventListener("pointerdown", (e) => {
    start = position(e);
    temp = h("div", { class: "rect" });
    box.append(temp);
    box.setPointerCapture(e.pointerId);
  });
  box.addEventListener("pointermove", (e) => {
    if (!start) return;
    const [x, y] = position(e);
    place(temp, Math.min(start[0], x), Math.min(start[1], y), Math.abs(x - start[0]), Math.abs(y - start[1]));
  });
  box.addEventListener("pointerup", (e) => {
    if (!start) return;
    const [x, y] = position(e);
    const rect = [Math.min(start[0], x), Math.min(start[1], y), Math.abs(x - start[0]), Math.abs(y - start[1])];
    start = null;
    temp.remove();
    if (rect[2] > 0.02 && rect[3] > 0.02 && rects.length < 10) { rects.push(rect); emit(); }
    draw();
  });
  const undo = h("button", { class: "btn small", text: t("Undo"), onclick: () => { rects.pop(); emit(); draw(); } });
  const clear = h("button", { class: "btn small", text: t("Clear all"), onclick: () => { rects = []; emit(); draw(); } });
  img.addEventListener("load", draw);
  draw();
  return h("div", { class: "stack" }, box, h("div", { class: "row" }, counter, h("div", { class: "row tight" }, undo, clear)));
}

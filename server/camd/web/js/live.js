import { api, busy, choiceDialog, detectionBox, fmtAgo, fmtDay, fmtTime, h, icon, labelName, prefs, saveToDevice, spinner, toast } from "./lib.js";
import { t } from "./text.js";

const REASONS = {
  privacy: "Privacy is on: the camera is off",
  snoozed: "Alerts are paused",
  manual: "Manual mode",
  home: "You are home: not recording",
  away: "You are away: watching",
  "auto-without-device": "No presence device: not recording",
};
const CLIP_OPTIONS = [[15, "15 s"], [30, "30 s"], [60, "1 min"], [120, "2 min"], [300, "5 min"]];
const POLL_MS = 4000;
const ACTIVE_POLL_MS = 1000;
const WAKE_WINDOW_MS = 25000;
const PLUG_WINDOW_MS = 60000;
const RETRY_CAP_MS = 15000;
const FREEZE_CHECK_MS = 4000;
const FREEZE_CHECKS = 3;

const clock = (s) => `${Math.floor(s / 60)}:${String(Math.max(0, Math.floor(s % 60))).padStart(2, "0")}`;

export function mount(el, navigate) {
  let destroyed = false;
  let pending = 0;
  let wakeUntil = 0;
  let wakeText = t("Turning the camera on...");
  let status = null;
  let wasClipping = false;
  const timers = new Set();
  const later = (fn, ms) => {
    const timer = setTimeout(() => { timers.delete(timer); if (!destroyed) fn(); }, ms);
    timers.add(timer);
    return timer;
  };

  const img = h("img", { alt: t("Live image"), draggable: "false" });
  const overlay = h("div", { class: "overlay" });
  const boxes = h("div", { class: "boxes" });
  const liveBadge = h("span", { class: "badge live", hidden: true }, h("span", { class: "pulse" }), t("LIVE"));
  const recBadge = h("span", { class: "badge rec", hidden: true }, h("span", { class: "pulse" }), t("RECORDING"));
  const fullBtn = h("button", { class: "round", "aria-label": t("Full screen"), title: t("Full screen (F)") }, icon("expand"));
  const clockHud = h("span", { class: "num" });
  const sizeHud = h("span", { class: "num" });
  const hud = h("div", { class: "hud", hidden: true }, clockHud, sizeHud);
  const viewer = h("div", { class: "viewer" }, img, boxes, overlay, h("div", { class: "badges" }, liveBadge, recBadge), hud, h("div", { class: "viewer-actions" }, fullBtn));
  const tickHud = () => {
    const now = new Date();
    clockHud.textContent = now.toLocaleTimeString(undefined, { hour12: false });
    sizeHud.textContent = img.naturalWidth ? `${img.naturalWidth}x${img.naturalHeight}` : "";
  };
  const hudTimer = setInterval(tickHud, 1000);

  let phase = "idle";
  let failures = 0;
  let retryTimer = null;
  let watcher = null;

  const showOverlay = (kind, text, action, sub) => {
    overlay.hidden = false;
    liveBadge.hidden = true;
    hud.hidden = true;
    overlay.replaceChildren(
      kind === "wait" ? spinner() : icon(kind === "standby" || kind === "off" ? "eyeOff" : "signal"),
      h("div", { class: "overlay-title", text }),
      sub ? h("div", { class: "overlay-sub", text: sub }) : null,
      action ? h("button", { class: "btn", text: action[0], onclick: action[1] }) : null
    );
  };

  function drawBoxes(items) {
    boxes.replaceChildren(...items.map(detectionBox));
  }

  function stopStream() {
    phase = "idle";
    clearInterval(watcher);
    clearTimeout(retryTimer);
    retryTimer = null;
    img.removeAttribute("src");
    boxes.replaceChildren();
  }

  function startStream() {
    if (phase !== "idle" || destroyed || document.hidden) return;
    if (!status || !status.camera.enabled || status.camera.standby || !status.camera.online) return;
    phase = "connecting";
    showOverlay("wait", failures ? t("Reconnecting...") : t("Connecting..."));
    img.src = "/api/live?t=" + Date.now();
    clearInterval(watcher);
    watcher = setInterval(() => {
      if (img.naturalWidth > 0) {
        phase = "playing";
        failures = 0;
        overlay.hidden = true;
        liveBadge.hidden = false;
        hud.hidden = false;
        tickHud();
        clearInterval(watcher);
      }
    }, 250);
  }

  const probe = document.createElement("canvas");
  probe.width = probe.height = 8;
  let lastSample = "";
  let stalledChecks = 0;
  const sampleImage = () => {
    try {
      const context = probe.getContext("2d", { willReadFrequently: true });
      context.drawImage(img, 0, 0, 8, 8);
      return context.getImageData(0, 0, 8, 8).data.join(",");
    } catch (_) {
      return "";
    }
  };
  const frozenWatch = setInterval(() => {
    if (phase !== "playing") { stalledChecks = 0; return; }
    const sample = sampleImage();
    stalledChecks = sample && sample === lastSample ? stalledChecks + 1 : 0;
    lastSample = sample;
    if (stalledChecks >= FREEZE_CHECKS) { stalledChecks = 0; stopStream(); startStream(); }
  }, FREEZE_CHECK_MS);

  img.addEventListener("error", () => {
    if (phase === "idle" || destroyed) return;
    stopStream();
    failures += 1;
    showOverlay("signal", t("No image"), [t("Retry"), () => { failures = 0; startStream(); }]);
    retryTimer = setTimeout(() => { retryTimer = null; startStream(); }, Math.min(RETRY_CAP_MS, 1000 * 2 ** Math.min(failures, 4)));
  });

  const onVisible = () => {
    if (document.hidden) stopStream();
    else { failures = 0; refresh().catch(() => {}); }
  };
  document.addEventListener("visibilitychange", onVisible);

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
  const onFullscreenChange = () => { if (!document.fullscreenElement && viewer.classList.contains("full")) setFull(false); };
  fullBtn.addEventListener("click", toggleFull);
  document.addEventListener("fullscreenchange", onFullscreenChange);

  const photoBtn = h("button", { class: "btn action" }, icon("snap"), h("span", { text: t("Photo") }));

  async function photoToServer() {
    const shot = await api("/api/photos", { method: "POST" });
    toast(t("Photo saved to Photos"), "ok", [t("View"), () => navigate(`/photos/${shot.id}`)]);
  }

  async function photoToDevice() {
    const res = await fetch("/api/snapshot", { credentials: "same-origin" });
    if (!res.ok) throw new Error(t("The camera is not sending images"));
    const done = await saveToDevice(await res.blob(), `snapshot-${Date.now()}.jpg`);
    if (done !== "cancelled") toast(done === "shared" ? t("Photo ready to save") : t("Photo downloaded"), "ok");
  }

  photoBtn.addEventListener("click", () => busy(photoBtn, async () => {
    try {
      let destination = prefs.photoDest;
      if (destination === "ask") {
        const pick = await choiceDialog(t("Save photo"), t("Choose where this photo goes."), [
          ["web", t("On the server"), t("It stays in the Photos gallery of the app"), "cloud"],
          ["device", t("On this device"), t("It is downloaded or shared from here"), "phone"],
        ], true);
        if (!pick) return;
        destination = pick.value;
        if (pick.remember) prefs.photoDest = destination;
      }
      await (destination === "web" ? photoToServer() : photoToDevice());
    } catch (e) {
      toast(e.message, "bad");
    }
  }));

  const clipSelect = h("select", { class: "select action-select", "aria-label": t("Clip length") },
    CLIP_OPTIONS.map(([value, text]) => h("option", { value, text, selected: value === prefs.clipSeconds })));
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
      clipBtn.replaceChildren(icon("record"), h("span", { text: t("Clip") }));
      clipBtn.title = t("Record a clip and keep it");
      return;
    }
    const paint = () => clipBtn.replaceChildren(icon("stop"), h("span", { text: t("Stop · {time}", { time: clock(Math.max(0, clip.until - Date.now() / 1000)) }) }));
    paint();
    clipTicker = setInterval(paint, 500);
    clipBtn.title = t("Stop and keep the clip");
  }

  clipBtn.addEventListener("click", () => busy(clipBtn, async () => {
    try {
      if (status && status.clip.active) await api("/api/clips/stop", { method: "POST" });
      else await api("/api/clips/start", { method: "POST", body: { seconds: Number(clipSelect.value) } });
    } catch (e) {
      toast(e.message, "bad");
    }
    await refresh().catch(() => {});
  }));

  const hero = h("div", { class: "card hero" });
  const modeButtons = [["auto", t("Auto")], ["armed", t("Armed")], ["disarmed", t("Disarmed")]].map(([id, label]) =>
    h("button", { text: label, "data-mode": id, onclick: () => act(() => api("/api/mode", { method: "POST", body: { mode: id } })) }));
  const segment = h("div", { class: "seg", role: "group", "aria-label": t("Mode") }, modeButtons);
  const hint = h("div", { class: "muted", hidden: true, text: t("Auto mode with no presence device: nothing is recorded until you arm it.") });
  const snoozeText = h("div", { class: "muted" });
  const snoozeButtons = h("div", { class: "row tight" });
  const privacySwitch = h("input", { type: "checkbox", class: "switch", "aria-label": t("Privacy") });
  let privacyBusy = false;
  const controls = h("div", { class: "card stack" },
    segment, hint, h("hr"),
    h("div", { class: "row" }, h("div", {}, h("div", { text: t("Pause alerts") }), snoozeText), snoozeButtons),
    h("hr"),
    h("div", { class: "row" }, h("div", {}, h("div", { text: t("Privacy") }), h("div", { class: "muted", text: t("Turns the camera sensor off") })), privacySwitch)
  );
  const last = h("div");

  const setCamera = (on) => act(async () => {
    await api("/api/settings", { method: "POST", body: { camera_enabled: on } });
    if (on) { wakeUntil = Date.now() + PLUG_WINDOW_MS; wakeText = t("Looking for the camera..."); failures = 0; }
    toast(on ? t("Camera enabled") : t("Camera disabled: nothing is recorded or alerted"), on ? "ok" : undefined);
  });

  async function act(fn) {
    pending += 1;
    try { await fn(); } catch (e) { toast(e.message, "bad"); }
    pending -= 1;
    await refresh().catch(() => {});
  }

  privacySwitch.addEventListener("change", async () => {
    const want = privacySwitch.checked;
    privacySwitch.disabled = true;
    privacyBusy = true;
    pending += 1;
    try {
      await api("/api/privacy", { method: "POST", body: { on: want } });
      if (want) {
        stopStream();
      } else {
        wakeUntil = Date.now() + WAKE_WINDOW_MS;
        wakeText = t("Turning the camera on...");
        failures = 0;
        showOverlay("wait", wakeText);
      }
    } catch (e) {
      privacySwitch.checked = !want;
      toast(e.message, "bad");
    }
    pending -= 1;
    privacyBusy = false;
    await refresh().catch(() => {});
  });

  function applyControls(s) {
    modeButtons.forEach((button) => {
      const on = button.dataset.mode === s.mode;
      button.classList.toggle("on", on);
      button.setAttribute("aria-pressed", String(on));
    });
    hint.hidden = !(!s.presence.configured && s.mode === "auto");
    const snoozed = s.snooze_until * 1000 > Date.now();
    const snooze = (minutes) => act(() => api("/api/snooze", { method: "POST", body: { minutes } }));
    snoozeText.textContent = snoozed ? t("Paused until {time}", { time: fmtTime(s.snooze_until) }) : t("Nothing is recorded or alerted");
    snoozeButtons.replaceChildren(...(snoozed
      ? [h("button", { class: "btn small", text: t("Resume"), onclick: () => snooze(0) })]
      : [[60, "1 h"], [480, "8 h"]].map(([minutes, text]) => h("button", { class: "btn small", text, onclick: () => snooze(minutes) }))));
    if (!privacyBusy) {
      privacySwitch.checked = s.privacy;
      privacySwitch.disabled = !s.camera.enabled;
    }
  }

  async function refresh() {
    const s = await api("/api/status");
    if (destroyed) return;
    status = s;
    recBadge.hidden = !s.recording;
    renderHero(hero, s);
    last.replaceChildren(lastEvent(s, navigate));
    if (!pending) applyControls(s);
    if (phase === "playing") drawBoxes(s.detection.items);

    const clipping = s.clip.active;
    if (wasClipping && !clipping) toast(t("Clip saved in Events"), "ok", [t("View"), () => navigate("/events")]);
    wasClipping = clipping;
    drawClip();
    const usable = s.camera.enabled && !s.camera.standby && s.camera.online;
    clipBtn.disabled = !clipping && !usable;
    photoBtn.disabled = !usable;

    if (!s.camera.enabled) {
      stopStream();
      showOverlay("off", t("Camera disabled"), [t("Enable camera"), () => setCamera(true)], t("Nothing is recorded or alerted. When you plug it in again, enable it and it comes back on its own."));
    } else if (s.camera.standby) {
      stopStream();
      showOverlay("standby", t("Camera off for privacy"), [t("Turn on"), () => { privacySwitch.checked = false; privacySwitch.dispatchEvent(new Event("change")); }]);
    } else if (!s.camera.online) {
      stopStream();
      if (Date.now() < wakeUntil) showOverlay("wait", wakeText, null, t("It can take half a minute if it has just been powered on."));
      else showOverlay("signal", t("Camera has no signal"), [t("Disable camera"), () => setCamera(false)], t("I keep looking for it every few seconds and it will come back on its own. If you unplugged it, disable it so it does not alert."));
    } else {
      wakeUntil = 0;
      startStream();
    }
  }

  function schedule() {
    if (destroyed) return;
    const active = status && (status.motion || status.recording);
    later(async () => {
      try { await refresh(); } catch (_) {}
      schedule();
    }, Date.now() < wakeUntil || active ? ACTIVE_POLL_MS : POLL_MS);
  }

  const onKey = (e) => {
    if (e.target.matches("input, select, textarea") || e.metaKey || e.ctrlKey || e.altKey || document.querySelector(".modal")) return;
    if (e.key === "f" || e.key === "F") toggleFull();
    else if (e.key === "s" || e.key === "S") photoBtn.click();
    else if (e.key === "Escape" && viewer.classList.contains("full")) setFull(false);
  };
  document.addEventListener("keydown", onKey);

  el.append(
    h("header", { class: "page-head" }, h("h1", { text: t("Live") })),
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
      clearInterval(frozenWatch);
      clearInterval(hudTimer);
      stopStream();
      document.removeEventListener("visibilitychange", onVisible);
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("fullscreenchange", onFullscreenChange);
      if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
      document.body.classList.remove("noscroll");
    },
  };
}

const fact = (label, value, tone) => h("div", { class: "fact" }, h("dt", { text: label }), h("dd", {}, tone ? h("span", { class: "dot " + tone }) : null, value));

function detectorFact(detection) {
  if (detection.mode === "off") return null;
  if (detection.state === "ready") return fact(t("Detector"), t("Active"), "ok");
  if (detection.state === "error") return fact(t("Detector"), t("Unavailable"), "bad");
  return fact(t("Detector"), t("Loading"), "warn");
}

function renderHero(hero, s) {
  if (!s.camera.enabled) {
    hero.className = "card hero off";
    hero.replaceChildren(
      h("div", { class: "hero-main" }, icon("eyeOff", "hero-ico"), h("div", {}, h("div", { class: "hero-title", text: t("Camera disabled") }), h("div", { class: "muted", text: t("Nothing is recorded or alerted until you enable it") })))
    );
    return;
  }
  const watching = s.armed;
  hero.className = "card hero " + (watching ? "on" : "off");
  const tone = s.camera.standby ? "warn" : s.camera.online ? "ok" : "bad";
  const text = s.camera.standby ? t("Standby") : s.camera.online ? t("Connected") : t("No signal");
  hero.replaceChildren(
    h("div", { class: "hero-main" },
      icon(watching ? "shield" : "shieldOff", "hero-ico"),
      h("div", {}, h("div", { class: "hero-title", text: watching ? t("Watching") : t("Not watching") }), h("div", { class: "muted", text: REASONS[s.reason] ? t(REASONS[s.reason]) : s.reason }))
    ),
    h("dl", { class: "facts" },
      fact(t("Camera"), text, tone),
      detectorFact(s.detection),
      s.motion && !s.recording ? fact(t("Motion"), t("Detected"), "warn") : null,
      s.camera.rssi ? fact(t("Signal"), `${s.camera.rssi} dBm`) : null,
      s.camera.fps ? fact(t("Frame rate"), `${s.camera.fps} fps`) : null
    )
  );
}

function lastEvent(s, navigate) {
  const event = s.last_event;
  if (!event) return h("div");
  const what = event.labels.length ? event.labels.map(labelName).join(", ") : t("Last motion");
  return h("button", { class: "card link-card", onclick: () => navigate(`/events/${event.id}`) },
    icon(event.labels.length ? "scan" : "film"),
    h("div", { class: "grow" }, h("div", { text: what }), h("div", { class: "muted", text: `${fmtDay(event.start)}, ${fmtTime(event.start)} · ${fmtAgo(event.start)}` })),
    icon("next", "dim")
  );
}

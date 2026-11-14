import { api, busy, choiceDialog, detectionBox, fmtTime, h, icon, prefs, saveToDevice, shareStatus, spinner, toast, watchState } from "./lib.js";
import { locale, t } from "./text.js";
import { eventItem, startOfDayTs, timeline } from "./timeline.js";

const REASONS = {
  privacy: "Privacy is on",
  snoozed: "Alerts are paused",
  manual: "Manual mode",
  home: "You are home",
  away: "You are away",
  "auto-without-device": "No presence device",
};
const RECENT = 3;
const CLIP_OPTIONS = [[15, "15 s"], [30, "30 s"], [60, "1 min"], [120, "2 min"], [300, "5 min"]];
const POLL_MS = 4000;
const ACTIVE_POLL_MS = 1000;
const WAKE_WINDOW_MS = 25000;
const PLUG_WINDOW_MS = 60000;
const RETRY_CAP_MS = 15000;
const FREEZE_CHECK_MS = 4000;
const FREEZE_CHECKS = 3;

const clock = (s) => `${Math.floor(s / 60)}:${String(Math.max(0, Math.floor(s % 60))).padStart(2, "0")}`;

export function mount(el, navigate, ctx = {}) {
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
  const liveTag = h("span", { class: "tag live", hidden: true }, h("span", { class: "pulse" }), t("Live"));
  const recTag = h("span", { class: "tag rec", hidden: true }, h("span", { class: "pulse" }), t("Recording"));
  const fullBtn = h("button", { class: "viewer-btn", "aria-label": t("Full screen"), title: t("Full screen (F)") }, icon("expand"));
  const clockHud = h("span", { class: "hud-clock num", hidden: true });
  const viewer = h("div", { class: "viewer" }, img, boxes, overlay, h("div", { class: "hud" }, h("div", { class: "tags" }, liveTag, recTag), clockHud), fullBtn);
  const tickHud = () => { clockHud.textContent = new Date().toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }); };
  const hudTimer = setInterval(tickHud, 1000);

  let phase = "idle";
  let failures = 0;
  let retryTimer = null;
  let watcher = null;

  const showOverlay = (kind, text, action, sub) => {
    overlay.hidden = false;
    liveTag.hidden = true;
    clockHud.hidden = true;
    overlay.replaceChildren(...[
      kind === "wait" ? spinner() : icon(kind === "standby" || kind === "off" ? "eyeOff" : "signal", "overlay-ico"),
      h("div", { class: "overlay-title", text }),
      sub ? h("div", { class: "overlay-sub", text: sub }) : null,
      action ? h("button", { class: "btn small", text: action[0], onclick: action[1] }) : null
    ].filter(Boolean));
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
        liveTag.hidden = false;
        clockHud.hidden = false;
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

  const clipSelect = h("select", { class: "clip-length", "aria-label": t("Clip length") },
    CLIP_OPTIONS.map(([value, text]) => h("option", { value, text, selected: value === prefs.clipSeconds })));
  clipSelect.addEventListener("change", () => { prefs.clipSeconds = Number(clipSelect.value); });
  const clipBtn = h("button", { class: "btn action" });
  const clipControl = h("div", { class: "clip-control" }, clipBtn, clipSelect);
  let clipTicker = null;

  function drawClip() {
    const clip = status && status.clip;
    const active = !!(clip && clip.active);
    clipControl.classList.toggle("recording", active);
    clipSelect.hidden = active;
    clearInterval(clipTicker);
    if (!active) {
      clipBtn.replaceChildren(icon("record"), h("span", { text: t("Clip") }));
      clipBtn.title = t("Record a clip and keep it");
      return;
    }
    const paint = () => clipBtn.replaceChildren(icon("stop"), h("span", { class: "num", text: t("Stop · {time}", { time: clock(Math.max(0, clip.until - Date.now() / 1000)) }) }));
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

  const stateDot = h("span", { class: "dot" });
  const stateText = h("span", { class: "state-text" });
  const reasonText = h("span", { class: "state-reason" });
  const telemetry = h("p", { class: "telemetry num" });

  const modeButtons = [["auto", t("Auto")], ["armed", t("Armed")], ["disarmed", t("Disarmed")]].map(([id, label]) =>
    h("button", { text: label, "data-mode": id, onclick: () => act(() => api("/api/mode", { method: "POST", body: { mode: id } })) }));
  const segment = h("div", { class: "seg", role: "group", "aria-label": t("Mode") }, modeButtons);
  const modeHint = h("p", { class: "group-note" });
  const snoozeText = h("div", { class: "setting-hint" });
  const snoozeButtons = h("div", { class: "row tight" });
  const privacySwitch = h("input", { type: "checkbox", class: "switch", "aria-label": t("Privacy") });
  let privacyBusy = false;
  const controls = h("section", { class: "group" },
    h("h2", { class: "group-title", text: t("Mode") }),
    h("div", { class: "group-body" },
      h("div", { class: "setting" }, segment),
      h("div", { class: "setting" }, h("div", { class: "setting-text" }, h("div", { class: "setting-label", text: t("Pause alerts") }), snoozeText), snoozeButtons),
      h("div", { class: "setting" }, h("div", { class: "setting-text" }, h("div", { class: "setting-label", text: t("Privacy") }), h("div", { class: "setting-hint", text: t("Turns the camera sensor off") })), privacySwitch)),
    modeHint);

  const todayTimeline = h("div");
  const recentList = h("div", { class: "event-rows" });
  const today = h("section", { class: "group today" },
    h("div", { class: "group-title row" }, h("h2", { text: t("Today") }), h("a", { class: "link", href: "#/events", text: t("All events") })),
    todayTimeline, recentList);
  let todayKey = null;

  async function loadToday(s) {
    const key = `${s.last_event ? s.last_event.id + ":" + s.last_event.status : "-"}:${new Date().toDateString()}`;
    if (key === todayKey) return;
    todayKey = key;
    const dayStart = startOfDayTs(0);
    let events = [];
    try { events = (await api(`/api/events?limit=100&hide_unconfirmed=true&after=${dayStart}`)).events; } catch (_) { todayKey = null; return; }
    if (destroyed) return;
    todayTimeline.replaceChildren(timeline(events, dayStart, (id) => navigate(`/events/${id}`), t("Events of the day"), { legend: false }));
    recentList.replaceChildren(...(events.length
      ? events.slice(0, RECENT).map((ev) => eventItem(ev, () => ev.status === "done" && navigate(`/events/${ev.id}`)))
      : [h("p", { class: "quiet-note", text: t("Nothing has happened today.") })]));
  }

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
    modeHint.textContent = !s.presence.configured && s.mode === "auto" ? t("Auto mode with no presence device: nothing is recorded until you arm it.")
      : s.mode === "auto" ? t("Auto records while you are away and stops when you come home.")
      : s.mode === "armed" ? t("Armed records and alerts all the time.")
      : t("Disarmed records nothing until you change the mode.");
    const snoozed = s.snooze_until * 1000 > Date.now();
    const snooze = (minutes) => act(() => api("/api/snooze", { method: "POST", body: { minutes } }));
    snoozeText.textContent = snoozed ? t("Paused until {time}", { time: fmtTime(s.snooze_until) }) : t("Nothing is recorded or alerted");
    snoozeButtons.replaceChildren(...(snoozed
      ? [h("button", { class: "btn small", text: t("Resume"), onclick: () => snooze(0) })]
      : [[60, "1 h"], [480, "8 h"]].map(([minutes, text]) => h("button", { class: "btn small num", text, onclick: () => snooze(minutes) }))));
    if (!privacyBusy) {
      privacySwitch.checked = s.privacy;
      privacySwitch.disabled = !s.camera.enabled;
    }
  }

  function applyState(s) {
    const cam = s.camera;
    const { tone, text } = watchState(s);
    stateDot.className = "dot " + tone;
    stateText.textContent = text;
    const snoozed = s.snooze_until * 1000 > Date.now();
    const reason = snoozed ? t("Paused until {time}", { time: fmtTime(s.snooze_until) }) : REASONS[s.reason] ? t(REASONS[s.reason]) : s.reason;
    const detector = s.detection.mode !== "off" && s.detection.state === "error" ? t("Detector unavailable") : "";
    reasonText.textContent = cam.enabled && !cam.standby ? [reason, detector].filter(Boolean).join(" · ") : "";
    const parts = cam.online && !cam.standby ? [
      cam.fps ? `${cam.fps} fps` : null,
      cam.rssi ? `${cam.rssi} dBm` : null,
      s.detection.mode !== "off" && s.detection.state === "ready" && s.detection.inference_ms ? t("Detector {ms} ms", { ms: Math.round(s.detection.inference_ms) }) : null,
    ].filter(Boolean) : [];
    telemetry.textContent = parts.join("  ·  ");
  }

  async function refresh() {
    const s = await api("/api/status");
    if (destroyed) return;
    status = s;
    shareStatus(s);
    recTag.hidden = !s.recording;
    applyState(s);
    loadToday(s);
    if (!pending) applyControls(s);
    if (phase === "playing") drawBoxes(s.detection.items);

    const clipping = s.clip.active;
    if (wasClipping && !clipping) toast(t("Clip saved in Events"), "ok", [t("View"), () => navigate("/events")]);
    wasClipping = clipping;
    drawClip();
    const usable = s.camera.enabled && !s.camera.standby && s.camera.online;
    clipBtn.disabled = !clipping && !usable;
    clipSelect.disabled = !usable;
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

  el.classList.add("live-page");
  el.append(
    h("header", { class: "page-head live-head" }, h("div", { class: "page-titles" }, h("h1", { text: ctx.siteName || t("Live") }), h("p", { class: "status-line" }, stateDot, stateText, reasonText))),
    h("div", { class: "live-layout" },
      h("div", { class: "stage" }, viewer, h("div", { class: "actions" }, photoBtn, clipControl, telemetry)),
      h("div", { class: "side" }, controls, today))
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

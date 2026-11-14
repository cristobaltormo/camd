import { authScreen, offlineScreen } from "./auth.js";
import { api, h, icon, isOffline, LoginRequired, on, spinner, store, toast, watchState } from "./lib.js";
import { t } from "./text.js";

export const BUILD = "dev";

const root = document.getElementById("app");

let siteName = "Camera";
let cameraList = [];

const CAMERA_COOKIE = "cam_camera";
const setCameraCookie = (id) => { document.cookie = `${CAMERA_COOKIE}=${encodeURIComponent(id)}; path=/; max-age=31536000; samesite=strict`; };

const VIEWS = {
  live: { label: "Live", load: () => import("./live.js") },
  events: { label: "Events", load: () => import("./events.js") },
  photos: { label: "Photos", load: () => import("./photos.js") },
  settings: { label: "Settings", load: () => import("./settings.js") },
};

const install = { event: null, available: () => !!install.event, async run() {
  if (!install.event) return;
  install.event.prompt();
  await install.event.userChoice;
  install.event = null;
} };
window.addEventListener("beforeinstallprompt", (e) => { e.preventDefault(); install.event = e; });
window.addEventListener("appinstalled", () => { install.event = null; toast(t("App installed"), "ok"); });

function registerWorker() {
  if (!("serviceWorker" in navigator)) return;
  const hadController = !!navigator.serviceWorker.controller;
  navigator.serviceWorker.register("/sw.js").catch(() => {});
  navigator.serviceWorker.addEventListener("controllerchange", () => {
    if (hadController) toast(t("There is a new version of the app"), "ok", [t("Reload"), () => location.reload()], true);
  });
}

async function hardReload() {
  try {
    const regs = await navigator.serviceWorker?.getRegistrations();
    for (const reg of regs || []) await reg.unregister();
    for (const key of await caches.keys()) await caches.delete(key);
  } catch (_) {}
  location.reload();
}

function checkBuild(serverBuild) {
  if (BUILD === "dev" || !serverBuild || serverBuild === BUILD) return;
  if (sessionStorage.getItem("cam.healed") !== serverBuild) {
    sessionStorage.setItem("cam.healed", serverBuild);
    hardReload();
    return;
  }
  toast(t("There is a new version of the app"), "ok", [t("Update"), () => hardReload()], true);
}

function navigate(path) {
  const target = "#" + path;
  if (location.hash === target) route();
  else location.hash = target;
}

let shell = null;
let current = null;
let routeSeq = 0;

function parseHash() {
  const legacy = location.hash.match(/^#event=(\d+)/);
  if (legacy) return ["events", legacy[1]];
  const [page, sub] = location.hash.replace(/^#\/?/, "").split("/");
  return [VIEWS[page] ? page : "live", sub];
}

async function route() {
  if (!shell) return;
  const [name, sub] = parseHash();
  shell.setActive(name);

  if (current && current.name === name) {
    await current.ready;
    if (current && current.name === name) current.view.update(sub);
    return;
  }

  const seq = ++routeSeq;
  if (current) { current.view.destroy(); current = null; }
  shell.view.replaceChildren(h("div", { class: "loading" }, spinner()));

  let mod;
  try {
    mod = await VIEWS[name].load();
  } catch (e) {
    console.error(e);
    if (seq === routeSeq) shell.view.replaceChildren(h("div", { class: "empty" }, icon("signal", "empty-ico"), h("h3", { text: t("The section could not be loaded") }), h("p", { text: t("This device may have an old version of the app stored.") }), h("button", { class: "btn primary", text: t("Update the app"), onclick: () => hardReload() })));
    return;
  }
  if (seq !== routeSeq) return;

  const fresh = h("div", { class: "page" });
  const view = mod.mount(fresh, navigate, { install, sub, siteName: currentName() });
  const entry = { name, view, ready: Promise.resolve(view.ready).catch(() => {}) };
  current = entry;
  try {
    await view.ready;
  } catch (e) {
    if (seq !== routeSeq || e instanceof LoginRequired) return;
    view.destroy();
    current = null;
    shell.view.replaceChildren(h("div", { class: "empty" }, icon("signal", "empty-ico"), h("h3", { text: e.message }), h("button", { class: "btn", text: t("Retry"), onclick: () => route() })));
    return;
  }
  if (seq !== routeSeq) return;
  shell.view.replaceChildren(fresh);
  view.update(sub);
}

function cameraSelect(cls) {
  return h("select", { class: cls, "aria-label": t("Camera"), onchange: (e) => { setCameraCookie(e.target.value); location.reload(); } },
    cameraList.map((camera) => h("option", { value: camera.id, text: camera.name, selected: camera.current })));
}

function buildShell() {
  const banner = h("div", { id: "offline", class: "banner", hidden: !isOffline(), role: "status" },
    h("span", { text: t("No connection to the server") }),
    h("button", { class: "link", text: t("Retry"), onclick: () => location.reload() }));
  const view = h("div", { class: "view" });
  const several = cameraList.length > 1;

  const links = Object.entries(VIEWS).map(([id, v]) =>
    h("a", { href: `#/${id}`, class: "nav-item", "data-page": id, title: t(v.label) }, icon(id), h("span", { class: "nav-label", text: t(v.label) }), h("span", { class: "badge-count num", hidden: true })));
  const statusDot = h("span", { class: "dot" });
  const statusText = h("span", { text: t("Connecting...") });

  const sidebar = h("aside", { class: "sidebar" },
    h("div", { class: "side-head" },
      several ? cameraSelect("camera-select") : h("div", { class: "side-name", text: currentName() }),
      h("div", { class: "side-status" }, statusDot, statusText)),
    h("nav", { class: "nav", "aria-label": t("Sections") }, links));
  const tabs = h("nav", { class: "tabs", "aria-label": t("Sections") }, h("div", {}, Object.entries(VIEWS).map(([id, v]) =>
    h("a", { href: `#/${id}`, class: "tab", "data-page": id }, icon(id), h("span", { text: t(v.label) }), h("span", { class: "badge-count num", hidden: true })))));

  const picker = several ? h("div", { class: "camera-bar" }, icon("camera"), cameraSelect("camera-select")) : null;

  root.replaceChildren(h("div", { class: "shell" }, sidebar, h("main", { class: "content" }, banner, picker, view), tabs));

  const api_ = {
    view,
    setActive(name) {
      document.querySelectorAll("[data-page]").forEach((el) => {
        const on_ = el.dataset.page === name;
        el.classList.toggle("on", on_);
        if (on_) el.setAttribute("aria-current", "page"); else el.removeAttribute("aria-current");
      });
      document.title = `${t(VIEWS[name].label)} · ${siteName}`;
    },
    setStatus(s) {
      const { tone, text } = watchState(s);
      statusDot.className = "dot " + tone;
      statusText.textContent = text;
    },
    setBadge(n) {
      document.querySelectorAll('[data-page="events"] .badge-count').forEach((b) => { b.hidden = !n; b.textContent = n > 99 ? "99+" : String(n); });
      if ("setAppBadge" in navigator) (n ? navigator.setAppBadge(n) : navigator.clearAppBadge()).catch(() => {});
    },
  };
  on("offline", (value) => { banner.hidden = !value; });
  return api_;
}

const currentName = () => cameraList.find((camera) => camera.current)?.name || siteName;

async function pollShell() {
  if (!shell) return;
  try {
    const s = await api("/api/status");
    checkBuild(s.build);
    shell.setStatus(s);
    const { count } = await api(`/api/events/count?since=${store.get("eventsSeen", Date.now() / 1000)}`);
    shell.setBadge(count);
  } catch (_) {}
}

async function enterApp() {
  const wanted = new URLSearchParams(location.search).get("camera");
  if (wanted) {
    setCameraCookie(wanted);
    history.replaceState(null, "", location.pathname + location.hash);
  }
  try { cameraList = (await api("/api/cameras")).cameras; } catch (_) { cameraList = []; }
  shell = buildShell();
  window.addEventListener("hashchange", route);
  window.addEventListener("cam:seen", () => shell && shell.setBadge(0));
  window.addEventListener("cam:status", (e) => shell && shell.setStatus(e.detail));
  document.addEventListener("keydown", (e) => {
    if (e.target.matches("input, select, textarea") || e.metaKey || e.ctrlKey || e.altKey || document.querySelector(".modal")) return;
    const target = { 1: "live", 2: "events", 3: "photos", 4: "settings" }[e.key];
    if (target) navigate(`/${target}`);
  });
  if (!store.get("eventsSeen", 0)) store.set("eventsSeen", Date.now() / 1000);
  route();
  pollShell();
  setInterval(pollShell, 10000);
}

async function start() {
  let state;
  try {
    state = await api("/api/auth/state");
  } catch (_) {
    root.replaceChildren(offlineScreen(siteName));
    return;
  }
  siteName = state.site_name;
  document.title = siteName;
  document.body.classList.toggle("auth", !state.logged_in);
  if (!state.logged_in) {
    root.replaceChildren(authScreen(state, () => location.reload()));
    return;
  }
  enterApp();
}

let lastReport = 0;
function report(reason) {
  if (reason instanceof LoginRequired) return;
  console.error(reason);
  if (Date.now() - lastReport < 5000) return;
  lastReport = Date.now();
  toast(t("Something went wrong. If it happens again, reload the app."), "bad");
}
window.addEventListener("unhandledrejection", (e) => report(e.reason));
window.addEventListener("error", (e) => report(e.error || e.message));

on("login", () => location.reload());
on("access", () => {
  const last = Number(sessionStorage.getItem("cam.accessReload") || 0);
  if (Date.now() - last < 15000) return toast(t("The access session has expired. Reload the page to sign in again."), "bad", [t("Reload"), () => location.reload()], true);
  sessionStorage.setItem("cam.accessReload", String(Date.now()));
  location.reload();
});
registerWorker();
start();

import { api, h, icon, isOffline, LoginRequired, on, spinner, store, toast } from "./lib.js?v=fd59e0f40c";
import { authScreen, offlineScreen } from "./auth.js?v=fd59e0f40c";

export const BUILD = "fd59e0f40c";

const root = document.getElementById("app");

const VIEWS = {
  live: { label: "En vivo", load: () => import("./live.js?v=fd59e0f40c") },
  events: { label: "Eventos", load: () => import("./events.js?v=fd59e0f40c") },
  photos: { label: "Fotos", load: () => import("./photos.js?v=fd59e0f40c") },
  settings: { label: "Ajustes", load: () => import("./settings.js?v=fd59e0f40c") },
};

const install = { event: null, available: () => !!install.event, async run() {
  if (!install.event) return;
  install.event.prompt();
  await install.event.userChoice;
  install.event = null;
} };
window.addEventListener("beforeinstallprompt", (e) => { e.preventDefault(); install.event = e; });
window.addEventListener("appinstalled", () => { install.event = null; toast("Aplicación instalada", "ok"); });

function registerWorker() {
  if (!("serviceWorker" in navigator)) return;
  const hadController = !!navigator.serviceWorker.controller;
  navigator.serviceWorker.register("/sw.js").catch(() => {});
  navigator.serviceWorker.addEventListener("controllerchange", () => {
    if (hadController) toast("Hay una versión nueva de la aplicación", "ok", ["Recargar", () => location.reload()], true);
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
  toast("Hay una versión nueva de la aplicación", "ok", ["Actualizar", () => hardReload()], true);
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
    if (seq === routeSeq) shell.view.replaceChildren(h("div", { class: "empty" }, icon("signal"), h("h3", { text: "No se pudo cargar la sección" }), h("p", { class: "muted", text: "Puede que el móvil tenga guardada una versión antigua de la aplicación." }), h("button", { class: "btn primary", text: "Actualizar la aplicación", onclick: () => hardReload() })));
    return;
  }
  if (seq !== routeSeq) return;

  const fresh = h("div", { class: "page" });
  const view = mod.mount(fresh, navigate, { install, sub });
  const entry = { name, view, ready: Promise.resolve(view.ready).catch(() => {}) };
  current = entry;
  try {
    await view.ready;
  } catch (e) {
    if (seq !== routeSeq || e instanceof LoginRequired) return;
    view.destroy();
    current = null;
    shell.view.replaceChildren(h("div", { class: "empty" }, icon("signal"), h("p", { text: e.message }), h("button", { class: "btn", text: "Reintentar", onclick: () => route() })));
    return;
  }
  if (seq !== routeSeq) return;
  shell.view.replaceChildren(fresh);
  view.update(sub);
}

function buildShell() {
  const banner = h("div", { id: "offline", class: "banner", hidden: !isOffline(), role: "status" },
    h("span", { text: "Sin conexión con el servidor" }),
    h("button", { class: "link", text: "Reintentar", onclick: () => location.reload() }));
  const view = h("div", { class: "view" });

  const links = Object.entries(VIEWS).map(([id, v]) => {
    const badge = h("span", { class: "badge-count", hidden: true });
    const a = h("a", { href: `#/${id}`, class: "nav-item", "data-page": id }, icon(id), h("span", { class: "nav-label", text: v.label }), badge);
    return { id, a, badge };
  });
  const statusDot = h("span", { class: "dot" });
  const statusText = h("span", { text: "Conectando..." });
  const statusSub = h("span", { class: "muted", text: "" });
  const status = h("div", { class: "side-status" }, statusDot, h("div", {}, statusText, statusSub));

  const sidebar = h("aside", { class: "sidebar" },
    h("div", { class: "brand" }, icon("logo", "brand-logo"), h("span", { class: "brand-name", text: "Habitación" })),
    h("nav", { class: "nav", "aria-label": "Secciones" }, links.map((l) => l.a)),
    status);
  const tabs = h("nav", { class: "tabs", "aria-label": "Secciones" }, h("div", {}, links.map((l) => {
    const clone = h("a", { href: `#/${l.id}`, class: "tab", "data-page": l.id }, icon(l.id), h("span", { text: VIEWS[l.id].label }), h("span", { class: "badge-count", hidden: true }));
    return clone;
  })));

  root.replaceChildren(h("div", { class: "shell" }, sidebar, h("main", { class: "content" }, banner, view), tabs));

  const api_ = {
    view,
    setActive(name) {
      document.querySelectorAll("[data-page]").forEach((el) => {
        const on_ = el.dataset.page === name;
        el.classList.toggle("on", on_);
        if (on_) el.setAttribute("aria-current", "page"); else el.removeAttribute("aria-current");
      });
      document.title = `${VIEWS[name].label} · Habitación`;
    },
    setStatus(s) {
      const tone = !s.camera.online && !s.camera.standby ? "bad" : s.camera.standby ? "warn" : s.armed ? "ok" : "";
      statusDot.className = "dot " + tone;
      statusText.textContent = s.camera.standby ? "Cámara apagada" : !s.camera.online ? "Cámara sin señal" : s.armed ? "Vigilando" : "No vigila";
      statusSub.textContent = s.camera.rssi ? `${s.camera.rssi} dBm` : "";
    },
    setBadge(n) {
      document.querySelectorAll('[data-page="events"] .badge-count').forEach((b) => { b.hidden = !n; b.textContent = n > 99 ? "99+" : String(n); });
      if ("setAppBadge" in navigator) (n ? navigator.setAppBadge(n) : navigator.clearAppBadge()).catch(() => {});
    },
  };
  on("offline", (value) => { banner.hidden = !value; });
  return api_;
}

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

function enterApp() {
  shell = buildShell();
  window.addEventListener("hashchange", route);
  window.addEventListener("cam:seen", () => shell && shell.setBadge(0));
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
    root.replaceChildren(offlineScreen());
    return;
  }
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
  toast("Algo ha fallado. Si se repite, recarga la aplicación.", "bad");
}
window.addEventListener("unhandledrejection", (e) => report(e.reason));
window.addEventListener("error", (e) => report(e.error || e.message));

on("login", () => location.reload());
on("access", () => {
  const last = Number(sessionStorage.getItem("cam.accessReload") || 0);
  if (Date.now() - last < 15000) return toast("La sesión de Cloudflare ha caducado. Recarga la página para entrar de nuevo.", "bad", ["Recargar", () => location.reload()], true);
  sessionStorage.setItem("cam.accessReload", String(Date.now()));
  location.reload();
});
registerWorker();
start();

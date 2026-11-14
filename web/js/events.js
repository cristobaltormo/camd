import { api, busy, confirmDialog, dayGroup, detectionBox, emptyState, fmtDay, fmtDur, fmtTime, h, icon, labelName, saveToDevice, store, toast } from "./lib.js";
import { count, locale, t } from "./text.js";
import { ANIMALS, endOfDayTs, eventItem, eventTitle, startOfDayTs, timeline } from "./timeline.js";

const PAGE = 60;
const POLL_MS = 15000;
const FILTERS = [["all", "All"], ["people", "People"], ["animals", "Animals"], ["clips", "Clips"], ["kept", "Kept"], ["noise", "No object"]];

export const isNoise = (ev) => ev.analyzed && ev.kind === "motion" && ev.labels.length === 0 && ev.status === "done";

export function matches(filter, ev) {
  if (filter === "noise") return isNoise(ev);
  if (isNoise(ev)) return false;
  if (filter === "people") return ev.labels.includes("person");
  if (filter === "animals") return ev.labels.some((label) => ANIMALS.has(label));
  if (filter === "clips") return ev.kind === "manual";
  if (filter === "kept") return ev.keep;
  return true;
}

export function reconcile(current, fresh, pageSize = PAGE) {
  const floor = fresh.length >= pageSize ? fresh[fresh.length - 1].start : -Infinity;
  return fresh.concat(current.filter((e) => e.start < floor));
}

export const queryFor = (filter) => (filter === "noise" ? "" : "&hide_unconfirmed=true") + (filter === "people" ? "&label=person" : "");

const signature = (events) => events.map((e) => `${e.id}:${e.status}:${e.keep ? 1 : 0}:${e.labels.join("+")}`).join(",");

export function mount(el, navigate) {
  const savedFilter = store.get("eventsFilter", "all");
  const st = { events: [], done: false, view: store.get("eventsView", "list"), dayOffset: 0, filter: FILTERS.some(([id]) => id === savedFilter) ? savedFilter : "all", selecting: false, picked: new Set(), modal: null };
  let destroyed = false;
  store.set("eventsSeen", Date.now() / 1000);
  window.dispatchEvent(new Event("cam:seen"));

  const counter = h("p", { class: "page-sub num" });
  const tabs = h("div", { class: "filters", role: "group", "aria-label": t("Filter") });
  const selectBtn = h("button", { class: "btn small ghost" });
  const viewButtons = [["list", t("List")], ["day", t("Day")]].map(([id, label]) => h("button", { text: label, "data-view": id, onclick: async () => {
    if (st.view === id) return;
    st.view = id;
    store.set("eventsView", id);
    try { await load(true); } catch (e) { toast(e.message, "bad"); }
    draw();
  } }));
  const viewSwitch = h("div", { class: "seg", role: "group", "aria-label": t("Events view") }, viewButtons);
  const dayNav = h("div", { class: "day-nav", hidden: true });
  const list = h("div", { class: "event-list" });
  const more = h("button", { class: "btn block more", text: t("Load more") });
  const bulk = h("div", { class: "bulkbar", hidden: true });

  el.append(
    h("header", { class: "page-head" }, h("div", { class: "page-titles" }, h("h1", { text: t("Events") }), counter), h("div", { class: "head-actions" }, viewSwitch, selectBtn)),
    tabs, dayNav, list, more, bulk
  );

  async function load(reset) {
    if (st.view === "day") {
      const dayStart = startOfDayTs(st.dayOffset);
      const { events } = await api(`/api/events?limit=200${queryFor(st.filter)}&after=${dayStart}&before=${endOfDayTs(dayStart)}`);
      if (destroyed) return;
      st.events = events;
      st.done = true;
      return;
    }
    const before = !reset && st.events.length ? st.events[st.events.length - 1].start : null;
    const { events } = await api(`/api/events?limit=${PAGE}${queryFor(st.filter)}` + (before ? `&before=${before}` : ""));
    if (destroyed) return;
    st.events = reset ? events : st.events.concat(events);
    st.done = events.length < PAGE;
  }

  const visible = () => st.events.filter((e) => matches(st.filter, e));
  const filtersInUse = () => FILTERS.filter(([id]) => id !== "noise" || st.filter === "noise" || st.events.some(isNoise));

  function drawTabs() {
    tabs.replaceChildren(...filtersInUse().map(([id, label]) =>
      h("button", { class: "filter" + (st.filter === id ? " on" : ""), text: t(label), "aria-pressed": String(st.filter === id), onclick: async () => {
        if (st.filter === id) return;
        st.filter = id;
        store.set("eventsFilter", id);
        try { await load(true); } catch (e) { toast(e.message, "bad"); }
        draw();
      } })
    ));
  }

  function drawDayNav() {
    dayNav.hidden = st.view !== "day";
    if (dayNav.hidden) return;
    const dayStart = startOfDayTs(st.dayOffset);
    const shift = async (delta) => { st.dayOffset = Math.max(0, st.dayOffset + delta); try { await load(true); } catch (e) { toast(e.message, "bad"); } draw(); };
    const date = new Date(dayStart * 1000).toLocaleDateString(locale(), { day: "numeric", month: "short" });
    dayNav.replaceChildren(
      h("div", { class: "day-switch" },
        h("button", { class: "btn small ghost icon-only", "aria-label": t("Previous day"), onclick: () => shift(1) }, icon("prev")),
        h("div", { class: "day-label" }, h("strong", { text: fmtDay(dayStart) }), st.dayOffset > 1 ? null : h("span", { class: "muted", text: date })),
        h("button", { class: "btn small ghost icon-only", "aria-label": t("Next day"), disabled: st.dayOffset === 0, onclick: () => shift(-1) }, icon("next"))),
      timeline(visible(), dayStart, (id) => navigate(`/events/${id}`), t("Events of the day"))
    );
  }

  function draw() {
    if (destroyed) return;
    drawTabs();
    drawDayNav();
    viewButtons.forEach((button) => { const on = button.dataset.view === st.view; button.classList.toggle("on", on); button.setAttribute("aria-pressed", String(on)); });
    const items = visible();
    counter.textContent = st.events.length ? count(items.length, "{n} event", "{n} events") + (st.done ? "" : "+") : "";
    more.hidden = st.done;
    selectBtn.textContent = st.selecting ? t("Done") : t("Select");
    selectBtn.disabled = !items.length && !st.selecting;
    list.classList.toggle("selecting", st.selecting);
    list.replaceChildren();
    if (!st.events.length) {
      list.append(st.view === "day"
        ? emptyState("events", t("Nothing on this day"), t("No motion was recorded and no clip was made."))
        : emptyState("events", t("No events yet"), t("Recordings appear here when motion is detected or you record a clip.")));
    } else if (!items.length) {
      list.append(emptyState("events", t("Nothing matches this filter"), st.done ? t("Try another filter.") : t("Load more events to look further back.")));
    }
    const groups = new Map();
    for (const ev of items) {
      const day = fmtDay(ev.start);
      if (!groups.has(day)) groups.set(day, []);
      groups.get(day).push(ev);
    }
    for (const [day, evs] of groups) {
      list.append(dayGroup(evs.length,
        st.view === "day" ? null : h("h2", { class: "day-head" }, h("span", { text: day }), h("span", { class: "num", text: String(evs.length) })),
        h("div", { class: "event-grid" }, evs.map(card))));
    }
    drawBulk();
  }

  function card(ev) {
    const picked = st.picked.has(ev.id);
    return eventItem(ev, () => {
      if (st.selecting) {
        if (ev.status === "recording") return;
        if (picked) st.picked.delete(ev.id); else st.picked.add(ev.id);
        draw();
      } else if (ev.status === "done") navigate(`/events/${ev.id}`);
    }, picked);
  }

  function drawBulk() {
    bulk.hidden = !st.selecting;
    if (!st.selecting) return;
    const n = st.picked.size;
    const selectable = visible().filter((e) => e.status !== "recording");
    const everything = n === selectable.length && n > 0;
    bulk.replaceChildren(
      h("span", { class: "num", text: n ? count(n, "{n} selected", "{n} selected") : t("Pick events") }),
      h("div", { class: "row tight" },
        h("button", { class: "btn small ghost", text: everything ? t("None") : t("All"), onclick: () => { st.picked = everything ? new Set() : new Set(selectable.map((e) => e.id)); draw(); } }),
        h("button", { class: "btn small danger-solid", disabled: !n, onclick: deleteSelected }, icon("trash"), t("Delete"))
      )
    );
  }

  async function deleteSelected() {
    const n = st.picked.size;
    const kept = st.events.filter((e) => st.picked.has(e.id) && e.keep).length;
    const extra = kept ? " " + count(kept, "It includes {n} protected event.", "It includes {n} protected events.") : "";
    if (!(await confirmDialog(count(n, "Delete {n} event", "Delete {n} events"), t("The recordings are removed from the server. This cannot be undone.") + extra, t("Delete"), true))) return;
    try {
      await api("/api/events/delete", { method: "POST", body: { ids: [...st.picked] } });
      st.events = st.events.filter((e) => !st.picked.has(e.id));
      st.picked = new Set();
      st.selecting = false;
      toast(t("Events deleted"));
      draw();
    } catch (e) { toast(e.message, "bad"); }
  }

  selectBtn.addEventListener("click", () => { st.selecting = !st.selecting; st.picked = new Set(); draw(); });
  more.addEventListener("click", () => busy(more, async () => {
    try { await load(false); draw(); } catch (e) { toast(e.message, "bad"); }
  }));

  function openPlayer(id) {
    closePlayer();
    const index = () => st.events.findIndex((e) => e.id === id);
    const done = (e) => e && e.status === "done";
    const go = (step) => {
      const next = st.events[index() + step];
      if (done(next)) navigate(`/events/${next.id}`);
    };
    const onKey = (e) => {
      if (document.querySelector(".center-modal")) return;
      if (e.key === "Escape") navigate("/events");
      else if (e.key === "ArrowLeft") go(1);
      else if (e.key === "ArrowRight") go(-1);
    };
    const i = index();
    const ev = i >= 0 ? st.events[i] : null;
    const older = done(st.events[i + 1]);
    const newer = i > 0 && done(st.events[i - 1]);

    const pinLabel = (keep) => [icon(keep ? "pinOn" : "pin"), h("span", { text: keep ? t("Protected") : t("Protect") })];
    const pinBtn = h("button", { class: "btn tool" + (ev && ev.keep ? " on" : ""), title: t("Protected events are never deleted automatically"), disabled: !ev }, ...pinLabel(ev && ev.keep));
    pinBtn.addEventListener("click", () => busy(pinBtn, async () => {
      const keep = !ev.keep;
      try {
        await api(`/api/events/${id}/keep`, { method: "POST", body: { keep } });
        ev.keep = keep;
        toast(keep ? t("Event protected: it will not be deleted automatically") : t("Protection removed"));
        pinBtn.classList.toggle("on", keep);
        pinBtn.replaceChildren(...pinLabel(keep));
        draw();
      } catch (e) { toast(e.message, "bad"); }
    }));

    const saveBtn = h("button", { class: "btn tool" }, icon("share"), h("span", { text: t("Save") }));
    saveBtn.addEventListener("click", () => busy(saveBtn, async () => {
      try {
        const how = await saveToDevice(`/api/events/${id}/clip`, `event-${id}.mp4`);
        if (how !== "cancelled") toast(how === "shared" ? t("Clip ready to save") : t("Clip downloaded"), "ok");
      } catch (e) { toast(e.message, "bad"); }
    }));

    const delBtn = h("button", { class: "btn tool danger" }, icon("trash"), h("span", { text: t("Delete") }));
    delBtn.addEventListener("click", () => busy(delBtn, async () => {
      const extra = ev && ev.keep ? " " + t("It is protected.") : "";
      if (!(await confirmDialog(t("Delete event"), t("The recording is removed from the server. This cannot be undone.") + extra, t("Delete"), true))) return;
      try { await api(`/api/events/${id}`, { method: "DELETE" }); } catch (e) { toast(e.message, "bad"); return; }
      const idx = index();
      const neighbour = [st.events[idx + 1], st.events[idx - 1]].find(done);
      st.events = st.events.filter((e) => e.id !== id);
      toast(t("Event deleted"));
      draw();
      navigate(neighbour ? `/events/${neighbour.id}` : "/events");
    }));

    const video = h("video", { controls: true, playsinline: true, autoplay: true, muted: true, preload: "metadata", src: `/api/events/${id}/clip`, poster: `/api/events/${id}/thumb` });
    video.addEventListener("error", () => toast(t("This video could not be loaded"), "bad"));
    const stage = h("div", { class: "player" }, video, ev && ev.detections.length ? h("div", { class: "boxes" }, ev.detections.map(detectionBox)) : null);
    const poster = new Image();
    poster.addEventListener("load", () => stage.style.setProperty("--ratio", String(poster.naturalWidth / poster.naturalHeight)));
    poster.src = `/api/events/${id}/thumb`;
    video.addEventListener("playing", () => stage.querySelector(".boxes")?.remove(), { once: true });

    const best = ev && ev.detections.length ? Math.max(...ev.detections.map((d) => d.confidence)) : 0;
    const subtitle = ev ? [`${fmtDay(ev.start)}, ${fmtTime(ev.start)}`, fmtDur(ev.duration), best ? t("{n}% sure", { n: Math.round(best * 100) }) : null].filter(Boolean).join(" · ") : "";
    const title = ev ? (ev.kind === "manual" ? t("Manual clip") : ev.labels.length ? ev.labels.map(labelName).join(", ") : eventTitle(ev)) : t("Event");
    const modal = h("div", { class: "modal sheet-modal", onclick: (e) => { if (e.target === modal) navigate("/events"); } },
      h("div", { class: "sheet media-sheet", role: "dialog", "aria-modal": "true", "aria-label": title },
        h("div", { class: "sheet-head" },
          h("div", { class: "sheet-titles" }, h("h2", { class: "sheet-title", text: title }), ev ? h("p", { class: "sheet-sub num", text: subtitle }) : null),
          h("button", { class: "btn ghost icon-only", "aria-label": t("Close"), title: t("Close (Esc)"), onclick: () => navigate("/events") }, icon("close"))),
        stage,
        h("div", { class: "sheet-bar" },
          h("div", { class: "row tight" },
            h("button", { class: "btn ghost icon-only", "aria-label": t("Older"), title: t("Older (left arrow)"), disabled: !older, onclick: () => go(1) }, icon("prev")),
            h("button", { class: "btn ghost icon-only", "aria-label": t("Newer"), title: t("Newer (right arrow)"), disabled: !newer, onclick: () => go(-1) }, icon("next"))),
          h("div", { class: "row tight" }, pinBtn, saveBtn, delBtn))
      )
    );
    document.addEventListener("keydown", onKey);
    document.body.classList.add("noscroll");
    document.body.append(modal);
    st.modal = { id, modal, onKey };
  }

  function closePlayer() {
    if (!st.modal) return;
    document.removeEventListener("keydown", st.modal.onKey);
    const video = st.modal.modal.querySelector("video");
    if (video) { video.pause(); video.removeAttribute("src"); video.load(); }
    st.modal.modal.remove();
    document.body.classList.remove("noscroll");
    st.modal = null;
  }

  async function poll() {
    if (destroyed || document.hidden || st.selecting) return;
    try {
      if (st.view === "day") {
        const before = signature(st.events);
        await load(true);
        if (signature(st.events) !== before) draw();
        return;
      }
      const { events } = await api(`/api/events?limit=${PAGE}${queryFor(st.filter)}`);
      if (destroyed) return;
      const merged = reconcile(st.events, events);
      if (signature(merged) !== signature(st.events)) {
        st.events = merged;
        st.picked = new Set([...st.picked].filter((id) => merged.some((e) => e.id === id)));
        draw();
      }
    } catch (_) {}
  }
  const timer = setInterval(poll, POLL_MS);
  const onVisible = () => { if (!document.hidden) poll(); };
  document.addEventListener("visibilitychange", onVisible);

  return {
    ready: load(true).then(draw),
    update(sub) {
      if (destroyed) return;
      if (sub && /^\d+$/.test(sub)) openPlayer(Number(sub));
      else closePlayer();
    },
    destroy() {
      destroyed = true;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
      closePlayer();
    },
  };
}

import { api, busy, confirmDialog, emptyState, fmtAgo, fmtDay, fmtDur, fmtTime, h, icon, labelName, saveToDevice, store, toast } from "./lib.js";
import { count, t } from "./text.js";

const PAGE = 60;
const POLL_MS = 15000;
const ANIMALS = new Set(["cat", "dog", "bird", "horse", "sheep", "cow", "bear"]);
const FILTERS = [["all", "All"], ["today", "Today"], ["yesterday", "Yesterday"], ["week", "7 days"], ["people", "People"], ["animals", "Animals"], ["clips", "Clips"], ["kept", "Kept"], ["noise", "No object"]];

const startOfDay = (offset = 0) => {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - offset);
  return d.getTime() / 1000;
};

export const isNoise = (ev) => ev.analyzed && ev.kind === "motion" && ev.labels.length === 0 && ev.status === "done";

export function matches(filter, ev) {
  if (filter === "noise") return isNoise(ev);
  if (isNoise(ev)) return false;
  if (filter === "today") return ev.start >= startOfDay(0);
  if (filter === "yesterday") return ev.start >= startOfDay(1) && ev.start < startOfDay(0);
  if (filter === "week") return ev.start >= startOfDay(6);
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

const labelBadge = (labels) => {
  if (!labels.length) return null;
  const extra = labels.length > 1 ? ` +${labels.length - 1}` : "";
  return h("span", { class: "tag" }, icon(ANIMALS.has(labels[0]) ? "paw" : "scan"), labelName(labels[0]) + extra);
};

export function mount(el, navigate) {
  const st = { events: [], done: false, filter: store.get("eventsFilter", "all"), selecting: false, picked: new Set(), modal: null };
  let destroyed = false;
  store.set("eventsSeen", Date.now() / 1000);
  window.dispatchEvent(new Event("cam:seen"));

  const counter = h("div", { class: "muted" });
  const chips = h("div", { class: "filters", role: "group", "aria-label": t("Filter") });
  const selectBtn = h("button", { class: "btn small" }, icon("select"), t("Select"));
  const list = h("div", { class: "event-list" });
  const more = h("button", { class: "btn block more", text: t("Load more") });
  const bulk = h("div", { class: "bulkbar", hidden: true });

  el.append(
    h("header", { class: "page-head" }, h("div", {}, h("h1", { text: t("Events") }), counter), h("div", { class: "row tight" }, selectBtn)),
    chips, list, more, bulk
  );

  async function load(reset) {
    const before = !reset && st.events.length ? st.events[st.events.length - 1].start : null;
    const { events } = await api(`/api/events?limit=${PAGE}${queryFor(st.filter)}` + (before ? `&before=${before}` : ""));
    if (destroyed) return;
    st.events = reset ? events : st.events.concat(events);
    st.done = events.length < PAGE;
  }

  const visible = () => st.events.filter((e) => matches(st.filter, e));
  const filtersInUse = () => FILTERS.filter(([id]) => id !== "noise" || st.events.some(isNoise));

  function drawChips() {
    if (!filtersInUse().some(([id]) => id === st.filter)) st.filter = "all";
    chips.replaceChildren(...filtersInUse().map(([id, label]) =>
      h("button", { class: "filter" + (st.filter === id ? " on" : ""), text: t(label), "aria-pressed": String(st.filter === id), onclick: async () => { st.filter = id; store.set("eventsFilter", id); try { await load(true); } catch (e) { toast(e.message, "bad"); } draw(); } })
    ));
  }

  function draw() {
    if (destroyed) return;
    drawChips();
    const items = visible();
    counter.textContent = st.events.length ? count(items.length, "{n} event", "{n} events") : "";
    more.hidden = st.done;
    selectBtn.replaceChildren(icon(st.selecting ? "close" : "select"), st.selecting ? t("Cancel") : t("Select"));
    list.classList.toggle("selecting", st.selecting);
    list.replaceChildren();
    if (!st.events.length) list.append(emptyState("film", t("No events yet"), t("Recordings appear here when motion is detected or you record a clip.")));
    else if (!items.length) list.append(emptyState("film", t("Nothing matches this filter"), st.done ? t("Try another filter.") : t("Load more events to look further back.")));
    let grid = null;
    let lastDay = "";
    for (const ev of items) {
      const day = fmtDay(ev.start);
      if (day !== lastDay) {
        list.append(h("div", { class: "day", text: day }));
        grid = h("div", { class: "grid" });
        list.append(grid);
        lastDay = day;
      }
      grid.append(card(ev));
    }
    drawBulk();
  }

  function card(ev) {
    const failed = ev.status === "failed";
    const picked = st.picked.has(ev.id);
    const media = ev.status === "done"
      ? h("div", { class: "thumb" }, h("img", { src: `/api/events/${ev.id}/thumb`, alt: "", loading: "lazy" }), labelBadge(ev.labels), h("span", { class: "dur", text: fmtDur(ev.duration) }), ev.keep ? h("span", { class: "pin" }, icon("pinOn")) : null)
      : h("div", { class: "thumb empty-thumb" }, icon(failed ? "signal" : "film"));
    const note = failed ? t("Could not be saved") : ev.status === "recording" ? t("Recording now") : (ev.kind === "manual" ? t("Clip") + " · " : "") + fmtAgo(ev.start);
    return h("button", {
      class: "event" + (picked ? " picked" : ""),
      "aria-label": t("Event at {time}", { time: fmtTime(ev.start) }),
      onclick: () => {
        if (st.selecting) {
          if (picked) st.picked.delete(ev.id); else st.picked.add(ev.id);
          draw();
        } else if (ev.status === "done") navigate(`/events/${ev.id}`);
      },
    }, media, h("span", { class: "tick" + (picked ? " on" : "") }, icon("check")), h("div", { class: "meta" }, h("strong", { text: fmtTime(ev.start) }), h("span", { class: "muted", text: note })));
  }

  function drawBulk() {
    bulk.hidden = !st.selecting;
    if (!st.selecting) return;
    const n = st.picked.size;
    const selectable = visible().filter((e) => e.status !== "recording");
    const everything = n === selectable.length && n > 0;
    bulk.replaceChildren(
      h("span", { text: n ? count(n, "{n} selected", "{n} selected") : t("Pick events") }),
      h("div", { class: "row tight" },
        h("button", { class: "btn small", text: everything ? t("None") : t("All"), onclick: () => { st.picked = everything ? new Set() : new Set(selectable.map((e) => e.id)); draw(); } }),
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

  function overlayBoxes(detections) {
    return h("div", { class: "boxes" }, detections.map((item) => {
      const [x, y, w, hh] = item.box;
      const frame = h("div", { class: "box" }, h("span", { text: `${labelName(item.label)} ${Math.round(item.confidence * 100)}%` }));
      Object.assign(frame.style, { left: x * 100 + "%", top: y * 100 + "%", width: w * 100 + "%", height: hh * 100 + "%" });
      return frame;
    }));
  }

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

    const pinBtn = h("button", { class: "btn small" + (ev && ev.keep ? " on" : ""), title: t("Protected events are never deleted automatically"), disabled: !ev }, icon(ev && ev.keep ? "pinOn" : "pin"), ev && ev.keep ? t("Protected") : t("Protect"));
    pinBtn.addEventListener("click", () => busy(pinBtn, async () => {
      const keep = !ev.keep;
      try {
        await api(`/api/events/${id}/keep`, { method: "POST", body: { keep } });
        ev.keep = keep;
        toast(keep ? t("Event protected: it will not be deleted automatically") : t("Protection removed"));
        pinBtn.classList.toggle("on", keep);
        pinBtn.replaceChildren(icon(keep ? "pinOn" : "pin"), keep ? t("Protected") : t("Protect"));
        draw();
      } catch (e) { toast(e.message, "bad"); }
    }));

    const saveBtn = h("button", { class: "btn small" }, icon("share"), t("To my device"));
    saveBtn.addEventListener("click", () => busy(saveBtn, async () => {
      try {
        const how = await saveToDevice(`/api/events/${id}/clip`, `event-${id}.mp4`);
        if (how !== "cancelled") toast(how === "shared" ? t("Clip ready to save") : t("Clip downloaded"), "ok");
      } catch (e) { toast(e.message, "bad"); }
    }));

    const delBtn = h("button", { class: "btn small danger" }, icon("trash"), t("Delete"));
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

    const video = h("video", { controls: true, playsinline: true, autoplay: true, preload: "metadata", src: `/api/events/${id}/clip`, poster: `/api/events/${id}/thumb` });
    video.addEventListener("error", () => toast(t("This video could not be loaded"), "bad"));
    const stage = h("div", { class: "player" }, video, ev && ev.detections.length ? overlayBoxes(ev.detections) : null);
    video.addEventListener("playing", () => stage.querySelector(".boxes")?.remove(), { once: true });

    const subtitle = ev ? [ev.kind === "manual" ? t("Manual clip") : null, fmtDur(ev.duration), fmtAgo(ev.start)].filter(Boolean).join(" · ") : "";
    const modal = h("div", { class: "modal sheet-modal", onclick: (e) => { if (e.target === modal) navigate("/events"); } },
      h("div", { class: "sheet stack", role: "dialog", "aria-modal": "true" },
        h("div", { class: "row" },
          h("div", {}, h("div", { class: "sheet-title", text: ev ? `${fmtDay(ev.start)}, ${fmtTime(ev.start)}` : t("Event") }), ev ? h("div", { class: "muted", text: subtitle }) : null),
          h("button", { class: "round flat", "aria-label": t("Close"), title: t("Close (Esc)"), onclick: () => navigate("/events") }, icon("close"))
        ),
        ev && ev.labels.length ? h("div", { class: "chips" }, ev.labels.map((label) => h("span", { class: "tag" }, icon(ANIMALS.has(label) ? "paw" : "scan"), labelName(label)))) : null,
        stage,
        h("div", { class: "row wrap" },
          h("div", { class: "row tight" },
            h("button", { class: "btn small", "aria-label": t("Older"), title: t("Older (left arrow)"), disabled: !older, onclick: () => go(1) }, icon("prev")),
            h("button", { class: "btn small", "aria-label": t("Newer"), title: t("Newer (right arrow)"), disabled: !newer, onclick: () => go(-1) }, icon("next"))
          ),
          h("div", { class: "row tight wrap" }, pinBtn, saveBtn, delBtn)
        )
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

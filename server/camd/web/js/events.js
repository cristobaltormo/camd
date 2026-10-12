import { api, busy, confirmDialog, emptyState, fmtAgo, fmtDay, fmtDur, fmtTime, h, icon, saveToDevice, store, toast } from "./lib.js?v=fd59e0f40c";

const PAGE = 60;
const POLL_MS = 15000;
const FILTERS = [["all", "Todos"], ["today", "Hoy"], ["yesterday", "Ayer"], ["week", "7 días"], ["clips", "Clips"], ["kept", "Guardados"]];

const startOfDay = (offset = 0) => {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - offset);
  return d.getTime() / 1000;
};

export function matches(filter, ev) {
  if (filter === "today") return ev.start >= startOfDay(0);
  if (filter === "yesterday") return ev.start >= startOfDay(1) && ev.start < startOfDay(0);
  if (filter === "week") return ev.start >= startOfDay(6);
  if (filter === "clips") return ev.kind === "manual";
  if (filter === "kept") return ev.keep;
  return true;
}

export function reconcile(current, fresh, pageSize = PAGE) {
  const floor = fresh.length >= pageSize ? fresh[fresh.length - 1].start : -Infinity;
  return fresh.concat(current.filter((e) => e.start < floor));
}

const signature = (events) => events.map((e) => `${e.id}:${e.status}:${e.keep ? 1 : 0}`).join(",");

export function mount(el, navigate) {
  const st = { events: [], done: false, filter: store.get("eventsFilter", "all"), selecting: false, picked: new Set(), modal: null };
  let destroyed = false;
  store.set("eventsSeen", Date.now() / 1000);
  window.dispatchEvent(new Event("cam:seen"));

  const count = h("div", { class: "muted" });
  const chips = h("div", { class: "filters", role: "group", "aria-label": "Filtrar" });
  const selectBtn = h("button", { class: "btn small" }, icon("select"), "Seleccionar");
  const list = h("div", { class: "event-list" });
  const more = h("button", { class: "btn block more", text: "Cargar más" });
  const bulk = h("div", { class: "bulkbar", hidden: true });

  el.append(
    h("header", { class: "page-head" }, h("div", {}, h("h1", { text: "Eventos" }), count), h("div", { class: "row tight" }, selectBtn)),
    chips, list, more, bulk
  );

  async function load(reset) {
    const before = !reset && st.events.length ? st.events[st.events.length - 1].start : null;
    const { events } = await api(`/api/events?limit=${PAGE}` + (before ? `&before=${before}` : ""));
    if (destroyed) return;
    st.events = reset ? events : st.events.concat(events);
    st.done = events.length < PAGE;
  }

  const visible = () => st.events.filter((e) => matches(st.filter, e));

  function drawChips() {
    chips.replaceChildren(...FILTERS.map(([id, label]) =>
      h("button", { class: "filter" + (st.filter === id ? " on" : ""), text: label, "aria-pressed": String(st.filter === id), onclick: () => { st.filter = id; store.set("eventsFilter", id); draw(); } })
    ));
  }

  function draw() {
    if (destroyed) return;
    drawChips();
    const items = visible();
    count.textContent = st.events.length ? `${items.length} ${items.length === 1 ? "evento" : "eventos"}` : "";
    more.hidden = st.done;
    selectBtn.replaceChildren(icon(st.selecting ? "close" : "select"), st.selecting ? "Cancelar" : "Seleccionar");
    list.classList.toggle("selecting", st.selecting);
    list.replaceChildren();
    if (!st.events.length) {
      list.append(emptyState("film", "Sin eventos todavía", "Aquí aparecerán las grabaciones cuando se detecte movimiento o grabes un clip."));
    } else if (!items.length) {
      list.append(emptyState("film", "Nada con este filtro", st.done ? "Prueba con otro filtro." : "Carga más eventos para buscar más atrás."));
    }
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
      ? h("div", { class: "thumb" }, h("img", { src: `/api/events/${ev.id}/thumb`, alt: "", loading: "lazy" }), h("span", { class: "dur", text: fmtDur(ev.duration) }), ev.keep ? h("span", { class: "pin" }, icon("pinOn")) : null)
      : h("div", { class: "thumb empty-thumb" }, icon(failed ? "signal" : "film"));
    const tick = h("span", { class: "tick" + (picked ? " on" : "") }, icon("check"));
    const note = failed ? "Error al guardar" : ev.status === "recording" ? "Grabando ahora" : (ev.kind === "manual" ? "Clip · " : "") + fmtAgo(ev.start);
    return h("button", {
      class: "event" + (picked ? " picked" : ""),
      "aria-label": `Evento de las ${fmtTime(ev.start)}`,
      onclick: () => {
        if (st.selecting) {
          if (st.picked.has(ev.id)) st.picked.delete(ev.id); else st.picked.add(ev.id);
          draw();
        } else if (ev.status === "done") navigate(`/events/${ev.id}`);
      },
    }, media, tick, h("div", { class: "meta" }, h("strong", { text: fmtTime(ev.start) }), h("span", { class: "muted", text: note })));
  }

  function drawBulk() {
    bulk.hidden = !st.selecting;
    if (!st.selecting) return;
    const n = st.picked.size;
    const all = visible().filter((e) => e.status !== "recording");
    bulk.replaceChildren(
      h("span", { text: n ? `${n} seleccionado${n > 1 ? "s" : ""}` : "Elige eventos" }),
      h("div", { class: "row tight" },
        h("button", { class: "btn small", text: n === all.length && n ? "Ninguno" : "Todos", onclick: () => { st.picked = n === all.length ? new Set() : new Set(all.map((e) => e.id)); draw(); } }),
        h("button", { class: "btn small danger-solid", disabled: !n, onclick: async () => {
          const kept = st.events.filter((e) => st.picked.has(e.id) && e.keep).length;
          const extra = kept ? ` Incluye ${kept} protegido${kept > 1 ? "s" : ""}.` : "";
          if (!(await confirmDialog(`Borrar ${n} evento${n > 1 ? "s" : ""}`, "Se eliminarán las grabaciones del servidor. No se puede deshacer." + extra, "Borrar", true))) return;
          try {
            await api("/api/events/delete", { method: "POST", body: { ids: [...st.picked] } });
            st.events = st.events.filter((e) => !st.picked.has(e.id));
            st.picked = new Set();
            st.selecting = false;
            toast("Eventos borrados");
            draw();
          } catch (e) { toast(e.message, "bad"); }
        } }, icon("trash"), "Borrar")
      )
    );
  }

  selectBtn.addEventListener("click", () => { st.selecting = !st.selecting; st.picked = new Set(); draw(); });
  more.addEventListener("click", () => busy(more, async () => {
    try { await load(false); draw(); } catch (e) { toast(e.message, "bad"); }
  }));

  // ---------- player ----------
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

    const pinBtn = h("button", { class: "btn small" + (ev && ev.keep ? " on" : ""), title: "Los eventos protegidos no se borran solos", disabled: !ev }, icon(ev && ev.keep ? "pinOn" : "pin"), ev && ev.keep ? "Protegido" : "Proteger");
    pinBtn.addEventListener("click", () => busy(pinBtn, async () => {
      const keep = !ev.keep;
      try {
        await api(`/api/events/${id}/keep`, { method: "POST", body: { keep } });
        ev.keep = keep;
        toast(keep ? "Evento protegido: no se borrará solo" : "Protección quitada");
        pinBtn.classList.toggle("on", keep);
        pinBtn.replaceChildren(icon(keep ? "pinOn" : "pin"), keep ? "Protegido" : "Proteger");
        draw();
      } catch (e) { toast(e.message, "bad"); }
    }));

    const saveBtn = h("button", { class: "btn small" }, icon("share"), "En mi dispositivo");
    saveBtn.addEventListener("click", () => busy(saveBtn, async () => {
      try {
        const how = await saveToDevice(`/api/events/${id}/clip`, `habitacion-${id}.mp4`);
        if (how !== "cancelled") toast(how === "shared" ? "Clip listo para guardar" : "Clip descargado", "ok");
      } catch (e) { toast(e.message, "bad"); }
    }));

    const delBtn = h("button", { class: "btn small danger" }, icon("trash"), "Borrar");
    delBtn.addEventListener("click", () => busy(delBtn, async () => {
      const extra = ev && ev.keep ? " Está protegido." : "";
      if (!(await confirmDialog("Borrar evento", "Se eliminará la grabación del servidor. No se puede deshacer." + extra, "Borrar", true))) return;
      try {
        await api(`/api/events/${id}`, { method: "DELETE" });
      } catch (e) { toast(e.message, "bad"); return; }
      const idx = index();
      const neighbour = [st.events[idx + 1], st.events[idx - 1]].find(done);
      st.events = st.events.filter((e) => e.id !== id);
      toast("Evento borrado");
      draw();
      navigate(neighbour ? `/events/${neighbour.id}` : "/events");
    }));

    const video = h("video", { controls: true, playsinline: true, autoplay: true, preload: "metadata", src: `/api/events/${id}/clip`, poster: `/api/events/${id}/thumb` });
    video.addEventListener("error", () => toast("No se pudo cargar este vídeo", "bad"));
    const modal = h("div", { class: "modal sheet-modal", onclick: (e) => { if (e.target === modal) navigate("/events"); } },
      h("div", { class: "sheet stack", role: "dialog", "aria-modal": "true" },
        h("div", { class: "row" },
          h("div", {}, h("div", { class: "sheet-title", text: ev ? `${fmtDay(ev.start)}, ${fmtTime(ev.start)}` : "Evento" }), ev ? h("div", { class: "muted", text: `${ev.kind === "manual" ? "Clip manual · " : ""}${fmtDur(ev.duration)} · ${fmtAgo(ev.start)}` }) : null),
          h("button", { class: "round flat", "aria-label": "Cerrar", title: "Cerrar (Esc)", onclick: () => navigate("/events") }, icon("close"))
        ),
        video,
        h("div", { class: "row wrap" },
          h("div", { class: "row tight" },
            h("button", { class: "btn small", "aria-label": "Más antiguo", title: "Más antiguo (flecha izquierda)", disabled: !older, onclick: () => go(1) }, icon("prev")),
            h("button", { class: "btn small", "aria-label": "Más reciente", title: "Más reciente (flecha derecha)", disabled: !newer, onclick: () => go(-1) }, icon("next"))
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

  // ---------- live updates ----------
  async function poll() {
    if (destroyed || document.hidden || st.selecting) return;
    try {
      const { events } = await api(`/api/events?limit=${PAGE}`);
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

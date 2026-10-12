import { api, confirmDialog, emptyState, fmtAgo, fmtDay, fmtDur, fmtTime, h, icon, store, toast } from "./lib.js";

const PAGE = 60;
const FILTERS = [["all", "Todos"], ["today", "Hoy"], ["yesterday", "Ayer"], ["week", "7 días"]];

const startOfDay = (offset = 0) => {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - offset);
  return d.getTime() / 1000;
};

function matches(filter, ev) {
  if (filter === "today") return ev.start >= startOfDay(0);
  if (filter === "yesterday") return ev.start >= startOfDay(1) && ev.start < startOfDay(0);
  if (filter === "week") return ev.start >= startOfDay(6);
  return true;
}

export function mount(el, navigate) {
  const st = { events: [], done: false, filter: store.get("eventsFilter", "all"), selecting: false, picked: new Set(), modal: null };
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
    drawChips();
    const items = visible();
    count.textContent = st.events.length ? `${items.length} ${items.length === 1 ? "evento" : "eventos"}` : "";
    more.hidden = st.done;
    selectBtn.replaceChildren(icon(st.selecting ? "close" : "select"), st.selecting ? "Cancelar" : "Seleccionar");
    list.classList.toggle("selecting", st.selecting);
    list.replaceChildren();
    if (!st.events.length) {
      list.append(emptyState("film", "Sin eventos todavía", "Aquí aparecerán las grabaciones cuando se detecte movimiento."));
    } else if (!items.length) {
      list.append(emptyState("film", "Nada en este periodo", st.done ? "Prueba con otro filtro." : "Carga más eventos para buscar más atrás."));
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
      ? h("div", { class: "thumb" }, h("img", { src: `/api/events/${ev.id}/thumb`, alt: "", loading: "lazy" }), h("span", { class: "dur", text: fmtDur(ev.duration) }))
      : h("div", { class: "thumb empty-thumb" }, icon(failed ? "signal" : "film"));
    const tick = h("span", { class: "tick" + (picked ? " on" : "") }, icon("check"));
    return h("button", {
      class: "event" + (picked ? " picked" : ""),
      "aria-label": `Evento de las ${fmtTime(ev.start)}`,
      onclick: () => {
        if (st.selecting) {
          if (st.picked.has(ev.id)) st.picked.delete(ev.id); else st.picked.add(ev.id);
          draw();
        } else if (ev.status === "done") navigate(`/events/${ev.id}`);
      },
    },
      media, tick,
      h("div", { class: "meta" },
        h("strong", { text: fmtTime(ev.start) }),
        h("span", { class: "muted", text: failed ? "Error al guardar" : ev.status === "recording" ? "Grabando ahora" : fmtAgo(ev.start) })
      )
    );
  }

  function drawBulk() {
    bulk.hidden = !st.selecting;
    if (!st.selecting) return;
    const n = st.picked.size;
    const all = visible().filter((e) => e.status !== "recording");
    bulk.replaceChildren(
      h("span", { text: n ? `${n} seleccionado${n > 1 ? "s" : ""}` : "Elige eventos" }),
      h("div", { class: "row tight" },
        h("button", { class: "btn small", text: n === all.length ? "Ninguno" : "Todos", onclick: () => { st.picked = n === all.length ? new Set() : new Set(all.map((e) => e.id)); draw(); } }),
        h("button", { class: "btn small danger-solid", disabled: !n, onclick: async () => {
          if (!(await confirmDialog(`Borrar ${n} evento${n > 1 ? "s" : ""}`, "Se eliminarán las grabaciones del servidor. No se puede deshacer.", "Borrar", true))) return;
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
  more.addEventListener("click", async () => {
    more.disabled = true;
    try { await load(false); draw(); } catch (e) { toast(e.message, "bad"); }
    more.disabled = false;
  });

  function openPlayer(id) {
    closePlayer();
    const index = () => st.events.findIndex((e) => e.id === id);
    const done = (e) => e && e.status === "done";
    const go = (step) => {
      const next = st.events[index() + step];
      if (done(next)) navigate(`/events/${next.id}`);
    };
    const onKey = (e) => {
      if (e.key === "Escape") navigate("/events");
      else if (e.key === "ArrowLeft") go(1);
      else if (e.key === "ArrowRight") go(-1);
    };
    const i = index();
    const ev = i >= 0 ? st.events[i] : null;
    const older = done(st.events[i + 1]);
    const newer = i > 0 && done(st.events[i - 1]);
    const video = h("video", { controls: true, playsinline: true, autoplay: true, src: `/api/events/${id}/clip`, poster: `/api/events/${id}/thumb` });
    const modal = h("div", { class: "modal sheet-modal", onclick: (e) => { if (e.target === modal) navigate("/events"); } },
      h("div", { class: "sheet stack", role: "dialog", "aria-modal": "true" },
        h("div", { class: "row" },
          h("div", {}, h("div", { class: "sheet-title", text: ev ? `${fmtDay(ev.start)}, ${fmtTime(ev.start)}` : "Evento" }), ev ? h("div", { class: "muted", text: `${fmtDur(ev.duration)} · ${fmtAgo(ev.start)}` }) : null),
          h("button", { class: "round flat", "aria-label": "Cerrar", title: "Cerrar (Esc)", onclick: () => navigate("/events") }, icon("close"))
        ),
        video,
        h("div", { class: "row wrap" },
          h("div", { class: "row tight" },
            h("button", { class: "btn small", "aria-label": "Más antiguo", title: "Más antiguo (flecha izquierda)", disabled: !older, onclick: () => go(1) }, icon("prev")),
            h("button", { class: "btn small", "aria-label": "Más reciente", title: "Más reciente (flecha derecha)", disabled: !newer, onclick: () => go(-1) }, icon("next"))
          ),
          h("div", { class: "row tight" },
            h("a", { class: "btn small", href: `/api/events/${id}/clip?download=1` }, icon("download"), "Descargar"),
            h("button", { class: "btn small danger", onclick: async () => {
              if (!(await confirmDialog("Borrar evento", "Se eliminará la grabación del servidor. No se puede deshacer.", "Borrar", true))) return;
              try {
                await api(`/api/events/${id}`, { method: "DELETE" });
                st.events = st.events.filter((e) => e.id !== id);
                toast("Evento borrado");
                navigate("/events");
              } catch (e) { toast(e.message, "bad"); }
            } }, icon("trash"), "Borrar")
          )
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
    st.modal.modal.querySelector("video")?.pause();
    st.modal.modal.remove();
    document.body.classList.remove("noscroll");
    st.modal = null;
  }

  const timer = setInterval(() => {
    if (st.selecting || st.modal) return;
    api(`/api/events?limit=${PAGE}`).then(({ events }) => {
      const known = new Set(st.events.map((e) => e.id));
      if (events.some((e) => !known.has(e.id) || e.status !== (st.events.find((x) => x.id === e.id) || {}).status)) {
        const tail = st.events.filter((e) => e.start < (events[events.length - 1] || { start: 0 }).start);
        st.events = events.concat(tail);
        draw();
      }
    }).catch(() => {});
  }, 15000);

  return {
    ready: load(true).then(draw),
    update(sub) {
      if (sub && /^\d+$/.test(sub)) openPlayer(Number(sub));
      else closePlayer();
    },
    destroy() {
      clearInterval(timer);
      closePlayer();
    },
  };
}

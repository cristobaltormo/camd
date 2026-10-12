import { api, busy, confirmDialog, emptyState, fmtAgo, fmtDay, fmtTime, h, icon, saveToDevice, toast } from "./lib.js?v=fd59e0f40c";

const PAGE = 60;

export function mount(el, navigate) {
  const st = { photos: [], done: false, selecting: false, picked: new Set(), modal: null };
  let destroyed = false;

  const count = h("div", { class: "muted" });
  const selectBtn = h("button", { class: "btn small" }, icon("select"), "Seleccionar");
  const shootBtn = h("button", { class: "btn small primary" }, icon("snap"), "Nueva foto");
  const list = h("div", { class: "photo-list" });
  const more = h("button", { class: "btn block more", text: "Cargar más" });
  const bulk = h("div", { class: "bulkbar", hidden: true });

  el.append(
    h("header", { class: "page-head" }, h("div", {}, h("h1", { text: "Fotos" }), count), h("div", { class: "row tight" }, selectBtn, shootBtn)),
    list, more, bulk
  );

  async function load(reset) {
    const before = !reset && st.photos.length ? st.photos[st.photos.length - 1].ts : null;
    const { photos } = await api(`/api/photos?limit=${PAGE}` + (before ? `&before=${before}` : ""));
    if (destroyed) return;
    st.photos = reset ? photos : st.photos.concat(photos);
    st.done = photos.length < PAGE;
  }

  function draw() {
    if (destroyed) return;
    count.textContent = st.photos.length ? `${st.photos.length}${st.done ? "" : "+"} ${st.photos.length === 1 ? "foto" : "fotos"}` : "";
    more.hidden = st.done;
    selectBtn.replaceChildren(icon(st.selecting ? "close" : "select"), st.selecting ? "Cancelar" : "Seleccionar");
    list.classList.toggle("selecting", st.selecting);
    list.replaceChildren();
    if (!st.photos.length) {
      list.append(emptyState("photos", "Sin fotos guardadas", "Haz una foto desde En vivo y elige guardarla en la web, o pulsa Nueva foto."));
    }
    let grid = null;
    let lastDay = "";
    for (const p of st.photos) {
      const day = fmtDay(p.ts);
      if (day !== lastDay) {
        list.append(h("div", { class: "day", text: day }));
        grid = h("div", { class: "grid" });
        list.append(grid);
        lastDay = day;
      }
      const picked = st.picked.has(p.id);
      grid.append(h("button", {
        class: "event photo" + (picked ? " picked" : ""),
        "aria-label": `Foto de las ${fmtTime(p.ts)}`,
        onclick: () => {
          if (st.selecting) {
            if (st.picked.has(p.id)) st.picked.delete(p.id); else st.picked.add(p.id);
            draw();
          } else navigate(`/photos/${p.id}`);
        },
      },
        h("div", { class: "thumb" }, h("img", { src: `/api/photos/${p.id}/thumb`, alt: "", loading: "lazy" })),
        h("span", { class: "tick" + (picked ? " on" : "") }, icon("check")),
        h("div", { class: "meta" }, h("strong", { text: fmtTime(p.ts) }), h("span", { class: "muted", text: fmtAgo(p.ts) }))
      ));
    }
    drawBulk();
  }

  function drawBulk() {
    bulk.hidden = !st.selecting;
    if (!st.selecting) return;
    const n = st.picked.size;
    bulk.replaceChildren(
      h("span", { text: n ? `${n} seleccionada${n > 1 ? "s" : ""}` : "Elige fotos" }),
      h("div", { class: "row tight" },
        h("button", { class: "btn small", text: n === st.photos.length && n ? "Ninguna" : "Todas", onclick: () => { st.picked = n === st.photos.length ? new Set() : new Set(st.photos.map((p) => p.id)); draw(); } }),
        h("button", { class: "btn small danger-solid", disabled: !n, onclick: async () => {
          if (!(await confirmDialog(`Borrar ${n} foto${n > 1 ? "s" : ""}`, "Se eliminarán del servidor. No se puede deshacer.", "Borrar", true))) return;
          try {
            await api("/api/photos/delete", { method: "POST", body: { ids: [...st.picked] } });
            st.photos = st.photos.filter((p) => !st.picked.has(p.id));
            st.picked = new Set();
            st.selecting = false;
            toast("Fotos borradas");
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
  shootBtn.addEventListener("click", () => busy(shootBtn, async () => {
    try {
      await api("/api/photos", { method: "POST" });
      await load(true);
      draw();
      toast("Foto guardada", "ok");
    } catch (e) { toast(e.message, "bad"); }
  }));

  function openViewer(id) {
    closeViewer();
    const index = () => st.photos.findIndex((p) => p.id === id);
    const go = (step) => {
      const next = st.photos[index() + step];
      if (next) navigate(`/photos/${next.id}`);
    };
    const onKey = (e) => {
      if (document.querySelector(".center-modal")) return;
      if (e.key === "Escape") navigate("/photos");
      else if (e.key === "ArrowLeft") go(1);
      else if (e.key === "ArrowRight") go(-1);
    };
    const i = index();
    const photo = i >= 0 ? st.photos[i] : null;
    const img = h("img", { class: "viewer-img", src: `/api/photos/${id}/img`, alt: "Foto" });
    img.addEventListener("error", () => toast("No se pudo cargar la foto", "bad"));

    const saveBtn = h("button", { class: "btn small" }, icon("share"), "En mi dispositivo");
    saveBtn.addEventListener("click", () => busy(saveBtn, async () => {
      try {
        const how = await saveToDevice(`/api/photos/${id}/img`, `habitacion-${id}.jpg`);
        if (how !== "cancelled") toast(how === "shared" ? "Foto lista para guardar" : "Foto descargada", "ok");
      } catch (e) { toast(e.message, "bad"); }
    }));
    const delBtn = h("button", { class: "btn small danger" }, icon("trash"), "Borrar");
    delBtn.addEventListener("click", () => busy(delBtn, async () => {
      if (!(await confirmDialog("Borrar foto", "Se eliminará del servidor. No se puede deshacer.", "Borrar", true))) return;
      try { await api(`/api/photos/${id}`, { method: "DELETE" }); } catch (e) { toast(e.message, "bad"); return; }
      const idx = index();
      const neighbour = st.photos[idx + 1] || st.photos[idx - 1];
      st.photos = st.photos.filter((p) => p.id !== id);
      toast("Foto borrada");
      draw();
      navigate(neighbour ? `/photos/${neighbour.id}` : "/photos");
    }));

    const modal = h("div", { class: "modal sheet-modal", onclick: (e) => { if (e.target === modal) navigate("/photos"); } },
      h("div", { class: "sheet stack", role: "dialog", "aria-modal": "true" },
        h("div", { class: "row" },
          h("div", {}, h("div", { class: "sheet-title", text: photo ? `${fmtDay(photo.ts)}, ${fmtTime(photo.ts)}` : "Foto" }), photo ? h("div", { class: "muted", text: fmtAgo(photo.ts) }) : null),
          h("button", { class: "round flat", "aria-label": "Cerrar", title: "Cerrar (Esc)", onclick: () => navigate("/photos") }, icon("close"))
        ),
        img,
        h("div", { class: "row wrap" },
          h("div", { class: "row tight" },
            h("button", { class: "btn small", "aria-label": "Más antigua", disabled: !st.photos[i + 1], onclick: () => go(1) }, icon("prev")),
            h("button", { class: "btn small", "aria-label": "Más reciente", disabled: i <= 0, onclick: () => go(-1) }, icon("next"))
          ),
          h("div", { class: "row tight wrap" }, saveBtn, delBtn)
        )
      )
    );
    document.addEventListener("keydown", onKey);
    document.body.classList.add("noscroll");
    document.body.append(modal);
    st.modal = { modal, onKey };
  }

  function closeViewer() {
    if (!st.modal) return;
    document.removeEventListener("keydown", st.modal.onKey);
    st.modal.modal.remove();
    document.body.classList.remove("noscroll");
    st.modal = null;
  }

  return {
    ready: load(true).then(draw),
    update(sub) {
      if (destroyed) return;
      if (sub && /^\d+$/.test(sub)) openViewer(Number(sub));
      else closeViewer();
    },
    destroy() {
      destroyed = true;
      closeViewer();
    },
  };
}

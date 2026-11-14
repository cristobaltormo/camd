import { api, busy, confirmDialog, dayGroup, emptyState, fmtAgo, fmtDay, fmtTime, h, icon, saveToDevice, toast } from "./lib.js";
import { count, t } from "./text.js";

const PAGE = 60;

export function mount(el, navigate) {
  const st = { photos: [], done: false, selecting: false, picked: new Set(), modal: null };
  let destroyed = false;

  const counter = h("p", { class: "page-sub num" });
  const selectBtn = h("button", { class: "btn small ghost" });
  const shootBtn = h("button", { class: "btn small" }, icon("snap"), h("span", { text: t("New photo") }));
  const list = h("div", { class: "photo-list" });
  const more = h("button", { class: "btn block more", text: t("Load more") });
  const bulk = h("div", { class: "bulkbar", hidden: true });

  el.append(
    h("header", { class: "page-head" }, h("div", { class: "page-titles" }, h("h1", { text: t("Photos") }), counter), h("div", { class: "head-actions" }, selectBtn, shootBtn)),
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
    counter.textContent = st.photos.length ? count(st.photos.length, "{n} photo", "{n} photos").replace(/(\d+)/, `$1${st.done ? "" : "+"}`) : "";
    more.hidden = st.done;
    selectBtn.textContent = st.selecting ? t("Done") : t("Select");
    selectBtn.disabled = !st.photos.length && !st.selecting;
    list.classList.toggle("selecting", st.selecting);
    list.replaceChildren();
    if (!st.photos.length) {
      list.append(emptyState("photos", t("No saved photos"), t("Take a photo from Live and keep it on the server, or press New photo.")));
    }
    const groups = new Map();
    for (const p of st.photos) {
      const day = fmtDay(p.ts);
      if (!groups.has(day)) groups.set(day, []);
      groups.get(day).push(p);
    }
    for (const [day, photos] of groups) {
      list.append(dayGroup(photos.length,
        h("h2", { class: "day-head" }, h("span", { text: day }), h("span", { class: "num", text: String(photos.length) })),
        h("div", { class: "photo-grid" }, photos.map(tile))));
    }
    drawBulk();
  }

  function tile(p) {
    const picked = st.picked.has(p.id);
    return h("button", {
      class: "photo" + (picked ? " picked" : ""),
      "aria-label": t("Photo at {time}", { time: fmtTime(p.ts) }),
      title: fmtTime(p.ts),
      onclick: () => {
        if (st.selecting) {
          if (st.picked.has(p.id)) st.picked.delete(p.id); else st.picked.add(p.id);
          draw();
        } else navigate(`/photos/${p.id}`);
      },
    },
      h("img", { src: `/api/photos/${p.id}/thumb`, alt: "", loading: "lazy", decoding: "async" }),
      h("span", { class: "tick" + (picked ? " on" : "") }, icon("check")));
  }

  function drawBulk() {
    bulk.hidden = !st.selecting;
    if (!st.selecting) return;
    const n = st.picked.size;
    bulk.replaceChildren(
      h("span", { class: "num", text: n ? t("{n} selected", { n }) : t("Pick photos") }),
      h("div", { class: "row tight" },
        h("button", { class: "btn small ghost", text: n === st.photos.length && n ? t("None") : t("All"), onclick: () => { st.picked = n === st.photos.length ? new Set() : new Set(st.photos.map((p) => p.id)); draw(); } }),
        h("button", { class: "btn small danger-solid", disabled: !n, onclick: async () => {
          if (!(await confirmDialog(count(n, "Delete {n} photo", "Delete {n} photos"), t("They are removed from the server. This cannot be undone."), t("Delete"), true))) return;
          try {
            await api("/api/photos/delete", { method: "POST", body: { ids: [...st.picked] } });
            st.photos = st.photos.filter((p) => !st.picked.has(p.id));
            st.picked = new Set();
            st.selecting = false;
            toast(t("Photos deleted"));
            draw();
          } catch (e) { toast(e.message, "bad"); }
        } }, icon("trash"), t("Delete"))
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
      toast(t("Photo saved"), "ok");
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
    const img = h("img", { class: "viewer-img", src: `/api/photos/${id}/img`, alt: t("Photo") });
    img.addEventListener("error", () => toast(t("The photo could not be loaded"), "bad"));

    const saveBtn = h("button", { class: "btn tool" }, icon("share"), h("span", { text: t("Save") }));
    saveBtn.addEventListener("click", () => busy(saveBtn, async () => {
      try {
        const how = await saveToDevice(`/api/photos/${id}/img`, `photo-${id}.jpg`);
        if (how !== "cancelled") toast(how === "shared" ? t("Photo ready to save") : t("Photo downloaded"), "ok");
      } catch (e) { toast(e.message, "bad"); }
    }));
    const delBtn = h("button", { class: "btn tool danger" }, icon("trash"), h("span", { text: t("Delete") }));
    delBtn.addEventListener("click", () => busy(delBtn, async () => {
      if (!(await confirmDialog(t("Delete photo"), t("It is removed from the server. This cannot be undone."), t("Delete"), true))) return;
      try { await api(`/api/photos/${id}`, { method: "DELETE" }); } catch (e) { toast(e.message, "bad"); return; }
      const idx = index();
      const neighbour = st.photos[idx + 1] || st.photos[idx - 1];
      st.photos = st.photos.filter((p) => p.id !== id);
      toast(t("Photo deleted"));
      draw();
      navigate(neighbour ? `/photos/${neighbour.id}` : "/photos");
    }));

    const modal = h("div", { class: "modal sheet-modal", onclick: (e) => { if (e.target === modal) navigate("/photos"); } },
      h("div", { class: "sheet media-sheet", role: "dialog", "aria-modal": "true", "aria-label": t("Photo") },
        h("div", { class: "sheet-head" },
          h("div", { class: "sheet-titles" }, h("h2", { class: "sheet-title", text: photo ? fmtDay(photo.ts) : t("Photo") }), photo ? h("p", { class: "sheet-sub num", text: `${fmtTime(photo.ts)} · ${fmtAgo(photo.ts)}` }) : null),
          h("button", { class: "btn ghost icon-only", "aria-label": t("Close"), title: t("Close (Esc)"), onclick: () => navigate("/photos") }, icon("close"))),
        h("div", { class: "player" }, img),
        h("div", { class: "sheet-bar" },
          h("div", { class: "row tight" },
            h("button", { class: "btn ghost icon-only", "aria-label": t("Older"), title: t("Older (left arrow)"), disabled: !st.photos[i + 1], onclick: () => go(1) }, icon("prev")),
            h("button", { class: "btn ghost icon-only", "aria-label": t("Newer"), title: t("Newer (right arrow)"), disabled: i <= 0, onclick: () => go(-1) }, icon("next"))),
          h("div", { class: "row tight" }, saveBtn, delBtn))
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

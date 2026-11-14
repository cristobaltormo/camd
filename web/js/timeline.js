import { fmtDur, fmtTime, h, icon, labelName } from "./lib.js";
import { t } from "./text.js";

const MIN_WIDTH = 0.004;
const DAY_S = 86400;
const GRID_HOURS = [3, 6, 9, 12, 15, 18, 21];
const LABEL_HOURS = [0, 6, 12, 18];
export const ANIMALS = new Set(["cat", "dog", "bird", "horse", "sheep", "cow", "bear"]);
const KIND_NAMES = { person: "People", object: "Animals and objects", motion: "Motion", clip: "Clips" };

export function startOfDayTs(offset = 0, now = new Date()) {
  const d = new Date(now);
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - offset);
  return d.getTime() / 1000;
}

export function endOfDayTs(dayStart) {
  const d = new Date(dayStart * 1000);
  d.setDate(d.getDate() + 1);
  return d.getTime() / 1000;
}

export function kindOf(ev) {
  if (ev.kind === "manual") return "clip";
  if (ev.labels.includes("person")) return "person";
  return ev.labels.length ? "object" : "motion";
}

export function eventTitle(ev) {
  if (ev.status === "failed") return t("Could not be saved");
  if (ev.status === "recording") return t("Recording now");
  if (ev.kind === "manual") return t("Clip");
  if (!ev.labels.length) return t("Motion");
  const names = ev.labels.slice(0, 2).map(labelName).join(", ");
  return ev.labels.length > 2 ? `${names} +${ev.labels.length - 2}` : names;
}

export function daySegments(events, dayStart) {
  const dayEnd = endOfDayTs(dayStart);
  const span = dayEnd - dayStart || DAY_S;
  return events
    .filter((ev) => ev.start >= dayStart && ev.start < dayEnd)
    .map((ev) => {
      const end = Math.min(ev.end || ev.start + (ev.duration || 0), dayEnd);
      const left = (ev.start - dayStart) / span;
      const width = Math.max(MIN_WIDTH, (end - ev.start) / span);
      return { id: ev.id, kind: kindOf(ev), left: Math.min(left, 1 - MIN_WIDTH), width, status: ev.status, start: ev.start };
    });
}

const at = (el, fraction) => { el.style.left = fraction * 100 + "%"; return el; };

export function timeline(events, dayStart, onOpen, label, { legend = true } = {}) {
  const dayEnd = endOfDayTs(dayStart);
  const span = dayEnd - dayStart || DAY_S;
  const segments = daySegments(events, dayStart);
  const track = h("div", { class: "timeline-track", role: "group", "aria-label": label },
    GRID_HOURS.map((hour) => at(h("span", { class: "grid-line" + (hour % 6 ? "" : " major") }), (hour * 3600) / DAY_S)));
  for (const seg of segments) {
    const mark = h("button", { class: `mark ${seg.kind}`, "data-id": String(seg.id), "aria-label": `${t(KIND_NAMES[seg.kind])} ${fmtTime(seg.start)}`, title: fmtTime(seg.start), onclick: () => seg.status === "done" && onOpen(seg.id) });
    at(mark, seg.left);
    mark.style.width = seg.width * 100 + "%";
    track.append(mark);
  }
  const now = Date.now() / 1000;
  if (now >= dayStart && now < dayEnd) track.append(at(h("span", { class: "now-line" }), (now - dayStart) / span));
  const hours = h("div", { class: "timeline-hours", "aria-hidden": "true" },
    LABEL_HOURS.map((hour) => at(h("span", { class: "num", text: `${String(hour).padStart(2, "0")}:00` }), hour / 24)));
  const kinds = Object.keys(KIND_NAMES).filter((kind) => segments.some((s) => s.kind === kind));
  const key = legend && kinds.length ? h("div", { class: "timeline-legend" }, kinds.map((kind) => h("span", {}, h("span", { class: `kind ${kind}` }), t(KIND_NAMES[kind])))) : null;
  return h("div", { class: "timeline" }, track, hours, key);
}

export function eventItem(ev, onClick, picked = false) {
  const done = ev.status === "done";
  const recording = ev.status === "recording";
  const thumb = done
    ? h("span", { class: "thumb" }, h("img", { src: `/api/events/${ev.id}/thumb`, alt: "", loading: "lazy", decoding: "async" }))
    : h("span", { class: "thumb placeholder" }, recording ? h("span", { class: "rec-dot" }) : icon("alert"));
  return h("button", { class: "event" + (picked ? " picked" : "") + (ev.status === "failed" ? " failed" : ""), "data-id": String(ev.id), "aria-label": t("Event at {time}", { time: fmtTime(ev.start) }), onclick: onClick },
    thumb,
    h("span", { class: "tick" + (picked ? " on" : "") }, icon("check")),
    h("span", { class: "info" },
      h("span", { class: "line" }, h("span", { class: "when num", text: fmtTime(ev.start) }), done ? h("span", { class: "dur num", text: fmtDur(ev.duration) }) : null),
      h("span", { class: "line" }, h("span", { class: "what" }, h("span", { class: `kind ${kindOf(ev)}` }), h("span", { class: "what-text", text: eventTitle(ev) })), ev.keep ? icon("pinOn", "kept") : null)));
}

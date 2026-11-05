import { h } from "./lib.js";

const MIN_WIDTH = 0.004;
const DAY_S = 86400;

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

export function daySegments(events, dayStart) {
  const dayEnd = endOfDayTs(dayStart);
  const span = dayEnd - dayStart || DAY_S;
  return events
    .filter((ev) => ev.start >= dayStart && ev.start < dayEnd)
    .map((ev) => {
      const end = Math.min(ev.end || ev.start + (ev.duration || 0), dayEnd);
      const left = (ev.start - dayStart) / span;
      const width = Math.max(MIN_WIDTH, (end - ev.start) / span);
      return { id: ev.id, kind: kindOf(ev), left: Math.min(left, 1 - MIN_WIDTH), width, status: ev.status };
    });
}

export function timeline(events, dayStart, onOpen, label) {
  const strip = h("div", { class: "timeline-strip", role: "group", "aria-label": label });
  for (const seg of daySegments(events, dayStart)) {
    const mark = h("button", { class: `mark ${seg.kind}`, "data-id": String(seg.id), "aria-label": `${label} ${seg.id}`, onclick: () => seg.status === "done" && onOpen(seg.id) });
    mark.style.left = seg.left * 100 + "%";
    mark.style.width = seg.width * 100 + "%";
    strip.append(mark);
  }
  const hours = h("div", { class: "timeline-hours" }, [0, 3, 6, 9, 12, 15, 18, 21, 24].map((hour) => h("span", { class: "num", text: String(hour).padStart(2, "0") })));
  return h("div", { class: "timeline" }, strip, hours);
}

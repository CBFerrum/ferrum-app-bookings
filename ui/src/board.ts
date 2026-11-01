// The schedule board's rules, without a page: how bookings become blocks (a class session is one
// block), how blocks that overlap share a column, where working hours, time off and buffers fall on
// a day, and what a drag lands on. The server decides every move; the hints here (a red block, "outside
// hours") only say, while the pointer is down, what it is likely to answer. Times are the
// workspace's: positions count minutes since the lane's midnight, which on a day the clocks change
// is 23 or 25 hours long.

import { addDays, clockText, dayBounds, dayOf, instantAt, weekday } from "./zone";

export type Status = "Pending" | "Confirmed" | "Waitlisted" | "Cancelled" | "No Show" | "Completed";

export interface Bk {
  id: string;
  version?: number;
  number?: string | null;
  service: string;
  service_name?: string | null;
  staff?: string | null;
  staff_name?: string | null;
  start_time: string;
  end_time: string;
  /** The booking with its buffers (what it holds on the staff member's calendar). */
  block_start?: string | null;
  block_end?: string | null;
  seats?: number | null;
  status: Status;
  resource?: string | null;
  session_key?: string | null;
  guest_name?: string | null;
  guest_email?: string | null;
  guest_phone?: string | null;
  source?: string | null;
  series?: string | null;
  amount?: string | null;
  customer?: string | null;
  sales_invoice?: string | null;
  location_type?: string | null;
  location?: string | null;
  notes?: string | null;
  cancellation_reason?: string | null;
  _titles?: Record<string, string>;
}

export interface ServiceRow {
  id: string;
  service_name: string;
  capacity?: number | null;
  duration_minutes?: number | null;
  active?: boolean;
}

export interface Hour {
  weekday: string;
  start_time: string;
  end_time: string;
}

export interface StaffRow {
  id: string;
  display_name: string;
  email: string;
  time_zone: string;
  active?: boolean;
  hours: Hour[];
}

export interface Off {
  id: string;
  staff: string;
  from_time: string;
  to_time: string;
  reason?: string | null;
}

export interface ResourceRow {
  id: string;
  resource_name: string;
  resource_type?: string | null;
  active?: boolean;
}

/** The colour a status wears on a badge. */
export const tone = (s: Status): "neutral" | "blue" | "green" | "amber" | "red" | "purple" =>
  s === "Pending" ? "amber" : s === "Confirmed" ? "blue" : s === "Completed" ? "green" : s === "No Show" ? "red" : s === "Waitlisted" ? "purple" : "neutral";

/** Statuses that hold time. */
export const HOLDING: Status[] = ["Pending", "Confirmed"];
export const holds = (b: { status: Status }) => HOLDING.includes(b.status);

// ── spans ──

export type Span = [number, number];

export function mergeSpans(spans: Span[]): Span[] {
  const out: Span[] = [];
  for (const s of spans.filter(([a, b]) => b > a).sort((x, y) => x[0] - y[0] || x[1] - y[1])) {
    const last = out[out.length - 1];
    if (last && s[0] <= last[1]) last[1] = Math.max(last[1], s[1]);
    else out.push([s[0], s[1]]);
  }
  return out;
}

export const clip = (spans: Span[], [from, to]: Span): Span[] => spans.map(([a, b]): Span => [Math.max(a, from), Math.min(b, to)]).filter(([a, b]) => b > a);

/** Minutes after midnight in `HH:MM` (24:00 allowed), or null. */
export function hhmm(v: string | null | undefined): number | null {
  const m = /^(\d{2}):(\d{2})$/.exec((v ?? "").trim());
  if (!m) return null;
  const [h, mi] = [Number(m[1]), Number(m[2])];
  return mi < 60 && (h < 24 || (h === 24 && mi === 0)) ? h * 60 + mi : null;
}

const NAMES = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/** When a staff member works between two instants: their weekly rows read on their own wall clock. */
export function workSpans(zone: string, hours: Hour[], from: number, to: number): Span[] {
  const out: Span[] = [];
  // A day either side, so a shift that crosses midnight in the staff member's zone stays whole.
  for (let day = addDays(dayOf(from, zone), -1); day <= addDays(dayOf(to, zone), 1); day = addDays(day, 1)) {
    const name = NAMES[weekday(day)];
    for (const h of hours.filter((x) => x.weekday === name)) {
      const [a, b] = [hhmm(h.start_time), hhmm(h.end_time)];
      if (a !== null && b !== null && b > a) out.push([instantAt(zone, day, a), instantAt(zone, day, b)]);
    }
  }
  return clip(mergeSpans(out), [from, to]);
}

// ── blocks ──

export interface Block {
  key: string;
  kind: "booking" | "session";
  /** The bookings that hold the block: one, or the seats of a class. */
  bookings: Bk[];
  staff: string | null;
  resource: string | null;
  start: number;
  end: number;
  /** With buffers. */
  from: number;
  to: number;
  status: Status;
  service: string;
  title: string;
  seats: number;
  capacity: number;
}

const ms = (v: string | null | undefined, fallback: number) => {
  const t = v ? Date.parse(v) : NaN;
  return Number.isNaN(t) ? fallback : t;
};

/** What a class session's status reads as when its seats differ: what still needs attention first. */
function sessionStatus(list: Bk[]): Status {
  for (const s of ["Confirmed", "Pending", "Completed", "No Show"] as const) if (list.some((b) => b.status === s)) return s;
  return list[0].status;
}

/** Bookings on the grid: one block each, except the seats of a class, which are one block together.
 *  Cancelled and waitlisted bookings hold no time, so they have none. */
export function blocks(bookings: Bk[], services: Map<string, ServiceRow>): Block[] {
  const groups = new Map<string, Bk[]>();
  for (const b of bookings.filter((x) => x.status !== "Cancelled" && x.status !== "Waitlisted")) {
    const key = b.session_key || b.id;
    groups.set(key, [...(groups.get(key) ?? []), b]);
  }
  return [...groups.entries()]
    .map(([key, list]) => {
      const first = list[0];
      const capacity = services.get(first.service)?.capacity ?? 1;
      const start = Math.min(...list.map((b) => ms(b.start_time, 0)));
      const end = Math.max(...list.map((b) => ms(b.end_time, start)));
      return {
        key,
        kind: capacity > 1 ? "session" : "booking",
        bookings: list,
        staff: first.staff ?? null,
        resource: first.resource ?? null,
        start,
        end,
        from: Math.min(...list.map((b) => ms(b.block_start, start))),
        to: Math.max(...list.map((b) => ms(b.block_end, end))),
        status: sessionStatus(list),
        service: first.service,
        title: capacity > 1 ? (first.service_name ?? "Class") : (first.guest_name ?? first.service_name ?? "Booking"),
        seats: list.reduce((n, b) => n + (b.seats ?? 1), 0),
        capacity,
      } satisfies Block;
    })
    .sort((a, b) => a.start - b.start || a.end - b.end);
}

export interface Placed<T> {
  item: T;
  col: number;
  cols: number;
}

/** Blocks that overlap in time stand side by side: each takes the first free column of its cluster,
 *  and the cluster's width is shared. `least` is the shortest a block is drawn (ms), so a 5-minute
 *  booking that is drawn taller than it lasts counts as overlapping what follows. */
export function pack<T extends { start: number; end: number }>(items: T[], least = 0): Placed<T>[] {
  const sorted = [...items].sort((a, b) => a.start - b.start || b.end - a.end);
  const out: Placed<T>[] = [];
  let cluster: Placed<T>[] = [];
  let ends: number[] = [];
  let reach = -Infinity;
  const close = () => {
    for (const p of cluster) p.cols = ends.length;
    out.push(...cluster);
    cluster = [];
    ends = [];
  };
  for (const item of sorted) {
    if (item.start >= reach) close();
    const shown = Math.max(item.end, item.start + least);
    let col = ends.findIndex((e) => e <= item.start);
    if (col < 0) col = ends.length;
    ends[col] = shown;
    reach = Math.max(reach, shown);
    cluster.push({ item, col, cols: 1 });
  }
  close();
  return out;
}

// ── lanes ──

export interface Lane {
  id: string;
  label: string;
  sub?: string;
  date: string;
  /** The lane's day, from local midnight to the next. */
  start: number;
  end: number;
  work: Span[];
  off: Span[];
}

export function makeLane(o: { id: string; label: string; sub?: string; date: string; zone: string; people: StaffRow[]; offs: Off[] }): Lane {
  const [start, end] = dayBounds(o.zone, o.date);
  const work = mergeSpans(o.people.flatMap((p) => workSpans(p.time_zone, p.hours, start, end)));
  // Time off is drawn for one person's lane; for several it would say nothing true about anyone.
  const mine = o.people.length === 1 ? o.offs.filter((t) => t.staff === o.people[0].id) : [];
  const off = clip(mergeSpans(mine.map((t): Span => [ms(t.from_time, 0), ms(t.to_time, 0)])), [start, end]);
  return { id: o.id, label: o.label, sub: o.sub, date: o.date, start, end, work, off };
}

/** Minutes of the day, from the lane's midnight. */
export const minutesIn = (lane: { start: number }, t: number) => (t - lane.start) / 60_000;

export interface Window {
  /** Minutes after midnight. */
  from: number;
  to: number;
}

/** The part of the day the board shows: the hours people work and the bookings fall in, to whole
 *  hours, never less than eight of them. With nothing to go on, the working day (8 to 6). */
export function viewWindow(lanes: Lane[], placed: Block[]): Window {
  const marks: number[] = [];
  for (const lane of lanes) {
    const last = minutesIn(lane, lane.end);
    for (const [a, b] of lane.work) marks.push(minutesIn(lane, a), minutesIn(lane, b));
    for (const blk of placed.filter((x) => x.from < lane.end && x.to > lane.start)) marks.push(Math.max(0, minutesIn(lane, blk.from)), Math.min(last, minutesIn(lane, blk.to)));
  }
  if (!marks.length) return { from: 480, to: 1080 };
  // The shortest day shown bounds the window, so a day the clocks shorten is never cut off.
  const length = Math.min(...lanes.map((l) => minutesIn(l, l.end)));
  let from = Math.max(0, Math.floor(Math.min(...marks) / 60) * 60);
  let to = Math.min(length, Math.ceil(Math.max(...marks) / 60) * 60);
  if (to - from < 480) to = Math.min(length, from + 480);
  if (to - from < 480) from = Math.max(0, to - 480);
  return { from, to };
}

/** The hour lines of a window, labelled as the wall clock reads there. */
export function hourMarks(lane: Lane, w: Window, zone: string, hour12: boolean): { at: number; label: string }[] {
  const out: { at: number; label: string }[] = [];
  for (let m = Math.ceil(w.from / 60) * 60; m < w.to; m += 60) out.push({ at: m, label: clockText(lane.start + m * 60_000, zone, hour12) });
  return out;
}

// ── dragging ──

export const STEP = 15;
export const snap = (minutes: number) => Math.round(minutes / STEP) * STEP;

export type Drop = { kind: "ok" } | { kind: "overlap"; with: string } | { kind: "hours" } | { kind: "off" };

/** What a drop is likely to meet, for the colour of the ghost. The server answers for real. */
export function dropHint(moving: Block, start: number, others: Block[], work: Span[], off: Span[], limited: boolean): Drop {
  const length = moving.end - moving.start;
  const from = start - (moving.start - moving.from);
  const to = start + length + (moving.to - moving.end);
  const clash = others.find((o) => o.key !== moving.key && holds(o) && o.from < to && from < o.to);
  if (clash) return { kind: "overlap", with: clash.title };
  if (off.some(([a, b]) => a < start + length && start < b)) return { kind: "off" };
  // Working hours are only known for a person's own lane.
  if (limited && !work.some(([a, b]) => a <= start && start + length <= b)) return { kind: "hours" };
  return { kind: "ok" };
}

export const dropText = (d: Drop): string =>
  d.kind === "overlap" ? `Overlaps ${d.with}` : d.kind === "hours" ? "Outside working hours" : d.kind === "off" ? "During time off" : "";

/** The start a block lands on when it is held `grab` minutes below its top and let go at `y` minutes
 *  from the top of the window, on the quarter hour. */
export function landing(lane: Lane, w: Window, y: number, grab: number): number {
  return lane.start + (w.from + snap(y - grab)) * 60_000;
}

export const initials = (name?: string | null) => {
  const parts = (name ?? "").trim().split(/\s+/).filter(Boolean);
  return ((parts[0]?.[0] ?? "?") + (parts.length > 1 ? parts[parts.length - 1][0] : "")).toUpperCase();
};

// ── what can be done to a booking ──

export type Step = "confirm" | "reschedule" | "cancel_booking" | "complete" | "mark_no_show";

/** The actions open to a booking now, as the server will allow them: finishing waits for the start. */
export function nextSteps(b: { status: Status; start_time: string }, now: number): Step[] {
  const started = Date.parse(b.start_time) <= now;
  switch (b.status) {
    case "Pending":
      return ["confirm", "reschedule", "cancel_booking", ...(started ? (["mark_no_show"] as const) : [])];
    case "Confirmed":
      return ["reschedule", "cancel_booking", ...(started ? (["complete", "mark_no_show"] as const) : [])];
    case "Waitlisted":
      return ["cancel_booking"];
    default:
      return [];
  }
}

/** A day's bookings in the order a person works through them: under way, then still to come, then done. */
export function inOrder<T extends { start_time: string; end_time: string }>(list: T[], now: number): { now: T[]; next: T[]; done: T[] } {
  const by = (a: T, b: T) => a.start_time.localeCompare(b.start_time) || a.end_time.localeCompare(b.end_time);
  const t = (v: string) => Date.parse(v);
  const sorted = [...list].sort(by);
  return {
    now: sorted.filter((b) => t(b.start_time) <= now && t(b.end_time) > now),
    next: sorted.filter((b) => t(b.start_time) > now),
    done: sorted.filter((b) => t(b.end_time) <= now),
  };
}

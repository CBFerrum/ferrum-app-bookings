// Time zones without a library: what a zone's wall clock reads at an instant, and the instant a
// wall clock names. Guests see times in their own zone and the team's board runs in the workspace's;
// the server decides every booking, so these only have to draw the same day it does (a day of 23
// or 25 hours included). Dates are `YYYY-MM-DD` strings, instants are milliseconds.

const DAY = 86_400_000;

const clocks = new Map<string, Intl.DateTimeFormat>();

function clock(zone: string): Intl.DateTimeFormat {
  let f = clocks.get(zone);
  if (!f) {
    f = new Intl.DateTimeFormat("en-US", { timeZone: zone, hourCycle: "h23", year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric", second: "numeric" });
    clocks.set(zone, f);
  }
  return f;
}

export interface Wall {
  y: number;
  m: number;
  d: number;
  h: number;
  mi: number;
}

/** The wall clock of `zone` at an instant. */
export function wall(ms: number, zone: string): Wall {
  const parts = clock(zone).formatToParts(ms);
  const n = (type: string) => Number(parts.find((p) => p.type === type)?.value ?? 0);
  // Some engines write midnight as 24 even with a 23-hour cycle.
  return { y: n("year"), m: n("month"), d: n("day"), h: n("hour") % 24, mi: n("minute") };
}

/** The wall clock read as if it were UTC: the number every comparison here is done in. */
const asUtc = (w: Wall) => Date.UTC(w.y, w.m - 1, w.d, w.h, w.mi);

/** How far the zone's clock is ahead of UTC at an instant. */
export function offset(ms: number, zone: string): number {
  return asUtc(wall(ms, zone)) - Math.floor(ms / 60_000) * 60_000;
}

/** The instant a wall clock names, or null when the clocks skip it. A repeated time is its first pass. */
function resolve(zone: string, local: number): number | null {
  // A day either side brackets any one change of the clocks.
  const tries = [local - offset(local - DAY, zone), local - offset(local + DAY, zone)];
  const fits = tries.filter((t) => asUtc(wall(t, zone)) === local);
  return fits.length ? Math.min(...fits) : null;
}

/** The instant `minutes` after midnight of `date` on the wall clock of `zone`. A time the clocks skip
 *  means the first moment after the gap (as the server reads working hours). */
export function instantAt(zone: string, date: string, minutes: number): number {
  const [y, m, d] = date.split("-").map(Number);
  const base = Date.UTC(y, m - 1, d) + minutes * 60_000;
  for (let shift = 0; shift <= 180; shift++) {
    const t = resolve(zone, base + shift * 60_000);
    if (t !== null) return t;
  }
  return base;
}

const two = (n: number) => String(n).padStart(2, "0");

/** The calendar day of an instant in `zone`. */
export function dayOf(ms: number, zone: string): string {
  const w = wall(ms, zone);
  return `${w.y}-${two(w.m)}-${two(w.d)}`;
}

/** The instants a calendar day starts and ends at in `zone`: 23 or 25 hours long when the clocks change. */
export function dayBounds(zone: string, date: string): [number, number] {
  return [instantAt(zone, date, 0), instantAt(zone, addDays(date, 1), 0)];
}

export function addDays(date: string, n: number): string {
  const [y, m, d] = date.split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, d + n)).toISOString().slice(0, 10);
}

/** 0 = Sunday. */
export function weekday(date: string): number {
  return new Date(`${date}T00:00:00Z`).getUTCDay();
}

/** The first day of the week `date` is in; `first` is the weekday weeks start on (0 = Sunday). */
export function weekStart(date: string, first: number): string {
  return addDays(date, -((weekday(date) - first + 7) % 7));
}

export function isDate(v: unknown): v is string {
  if (typeof v !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(v)) return false;
  const at = new Date(`${v}T00:00:00Z`);
  return !Number.isNaN(at.getTime()) && at.toISOString().startsWith(v);
}

/** `YYYY-MM` of a date, and the months around it. */
export const monthOf = (date: string) => date.slice(0, 7);

export function addMonths(month: string, n: number): string {
  const [y, m] = month.split("-").map(Number);
  const at = new Date(Date.UTC(y, m - 1 + n, 1));
  return `${at.getUTCFullYear()}-${two(at.getUTCMonth() + 1)}`;
}

export function monthEnd(month: string): string {
  return addDays(`${addMonths(month, 1)}-01`, -1);
}

/** The weeks that show a month: every date of it, with the days of the neighbouring months that fill the first and last week. */
export function monthWeeks(month: string, first: number): string[][] {
  const start = weekStart(`${month}-01`, first);
  const last = monthEnd(month);
  const weeks: string[][] = [];
  for (let day = start; day <= last; day = addDays(day, 7)) weeks.push(Array.from({ length: 7 }, (_, i) => addDays(day, i)));
  return weeks;
}

// ── showing them ──

const formats = new Map<string, Intl.DateTimeFormat>();

function format(zone: string, options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = `${zone}|${JSON.stringify(options)}`;
  let f = formats.get(key);
  if (!f) {
    f = new Intl.DateTimeFormat(undefined, { ...options, timeZone: zone });
    formats.set(key, f);
  }
  return f;
}

/** `10:30` or `10:30 AM`, as the reader keeps time. */
export function clockText(ms: number, zone: string, hour12: boolean): string {
  // hourCycle, not hour12: false, which en-US writes as 24:00 at midnight.
  return format(zone, { hour: hour12 ? "numeric" : "2-digit", minute: "2-digit", hourCycle: hour12 ? "h12" : "h23" }).format(ms);
}

/** A date such as `Thu 12 Nov` (the order is the reader's own). */
export function dayText(date: string, options: Intl.DateTimeFormatOptions = { weekday: "short", day: "numeric", month: "short" }): string {
  return format("UTC", options).format(Date.parse(`${date}T00:00:00Z`));
}

/** `Thursday 12 November 2026, 10:00` in a zone. */
export function whenText(ms: number, zone: string, hour12: boolean): string {
  return `${format(zone, { weekday: "long", day: "numeric", month: "long", year: "numeric" }).format(ms)}, ${clockText(ms, zone, hour12)}`;
}

/** `Asia/Kolkata (GMT+5:30)`. */
export function zoneText(zone: string, at = Date.now()): string {
  try {
    const off = format(zone, { timeZoneName: "shortOffset" }).formatToParts(at).find((p) => p.type === "timeZoneName")?.value;
    return off ? `${zone.replace(/_/g, " ")} (${off})` : zone;
  } catch {
    return zone;
  }
}

export function isZone(zone: string): boolean {
  try {
    clock(zone);
    return true;
  } catch {
    return false;
  }
}

/** The zones this browser knows, the ones that matter first. */
export function zoneList(first: string[]): string[] {
  const all = (Intl as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf?.("timeZone") ?? [];
  const head = first.filter((z, i) => z && isZone(z) && first.indexOf(z) === i);
  return [...head, ...all.filter((z) => !head.includes(z))];
}

export function browserZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

/** Whether the reader's clock has an AM/PM. */
export function prefers12Hour(): boolean {
  try {
    return new Intl.DateTimeFormat(undefined, { hour: "numeric" }).resolvedOptions().hour12 === true;
  } catch {
    return false;
  }
}

/** The weekday weeks start on for the reader (0 = Sunday); Monday where the browser cannot say. */
export function firstWeekday(): number {
  try {
    const l = new Intl.Locale(navigator.language) as Intl.Locale & { getWeekInfo?: () => { firstDay: number }; weekInfo?: { firstDay: number } };
    const day = (l.getWeekInfo?.() ?? l.weekInfo)?.firstDay;
    return day ? day % 7 : 1;
  } catch {
    return 1;
  }
}

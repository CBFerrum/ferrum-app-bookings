// What the public booking page knows how to do without a browser: the shapes the site's endpoints
// answer, the days that have times, what a price and a length of time read like, the answers a
// form needs checked, the server's complaints put beside the field they are about, and the links
// that put a booking in a guest's calendar. Pure, so it is tested without a page.

import { ApiError, t } from "@ferrum/client";

export type AnswerType = "Text" | "Long Text" | "Number" | "Yes/No" | "Choice";

export interface Question {
  question: string;
  type: AnswerType;
  options: string[];
  required: boolean;
}

export interface Person {
  id: string;
  name: string;
  bio?: string | null;
}

export interface Service {
  id: string;
  name: string;
  description?: string | null;
  duration_minutes: number;
  seats: number;
  price: string;
  location_type: "In Person" | "Video" | "Phone";
  location?: string | null;
  staff: Person[];
  questions: Question[];
  /** How many in a row a guest may book it (0: once). */
  repeats?: number;
}

export interface Catalog {
  time_zone: string;
  currency: string | null;
  services: Service[];
}

export interface Slot {
  start: string;
  end: string;
  time: string;
  seats_left: number;
  /** A class with no room for these seats: its waitlist can be joined. */
  full?: boolean;
  staff: { id: string; name: string }[];
}

export interface Slots {
  time_zone: string;
  from: string;
  to: string;
  days: { date: string; slots: Slot[] }[];
}

/** What a guest sees of their booking (`guest_view` in the app's site). */
export interface GuestBooking {
  number?: string | null;
  status: "Pending" | "Confirmed" | "Waitlisted" | "Cancelled" | "No Show" | "Completed" | "Expired";
  service: { id: string; name: string };
  staff: { id: string | null; name: string | null };
  start_time: string;
  end_time: string;
  time_zone: string;
  seats: number;
  location_type?: string | null;
  location?: string | null;
  guest: { name?: string | null; email?: string | null; phone?: string | null };
  answers: { question: string; answer: string }[];
  amount?: string | null;
  cancellation_reason?: string | null;
  waitlist_position?: number | null;
  can_cancel: boolean;
  cancel_until?: string | null;
  can_reschedule: boolean;
  /** The staff member's outside calendar became busy at this time, and nobody has kept it. */
  clash?: boolean;
  reschedule_until?: string | null;
}

/** The days that have times, by date. */
export function openDays(slots: Slots | undefined): Map<string, Slot[]> {
  return new Map((slots?.days ?? []).filter((d) => d.slots.length).map((d) => [d.date, d.slots]));
}

/** The earliest date of a set of days, or null. */
export function firstDay(open: Map<string, unknown>): string | null {
  return [...open.keys()].sort()[0] ?? null;
}

/** The person to book with when the guest left it to us: the one the slot names, or nobody. */
export function onlyPerson(slot: Slot): string | undefined {
  return slot.staff.length === 1 ? slot.staff[0].id : undefined;
}

export function priceText(price: string | number | null | undefined, currency: string | null): string {
  const n = Number(price ?? 0);
  if (!(n > 0)) return t("Free");
  try {
    return currency ? new Intl.NumberFormat(undefined, { style: "currency", currency }).format(n) : new Intl.NumberFormat().format(n);
  } catch {
    return `${currency ?? ""} ${n}`.trim();
  }
}

export function durationText(minutes: number): string {
  if (minutes < 60) return t("{minutes} min", { minutes });
  const [h, m] = [Math.floor(minutes / 60), minutes % 60];
  return m ? t("{h} h {m} min", { h, m }) : h === 1 ? "1 hour" : t("{h} hours", { h });
}

/** How the service meets: its address when it is in person; video links and numbers come after booking. */
export function whereText(type: string | null | undefined, location?: string | null): string {
  if (type === "Video") return t("Video call");
  if (type === "Phone") return t("Phone call");
  return location ? t("In person · {location}", { location }) : t("In person");
}

export const isLink = (v: string | null | undefined): v is string => !!v && /^https?:\/\/\S+$/i.test(v.trim());

// ── the form ──

export interface Form {
  name: string;
  email: string;
  phone: string;
  notes: string;
  seats: number;
  /** Answers by question text. */
  answers: Record<string, string>;
  /** Booked again at the same time: "" (once), "week", "2 weeks", "4 weeks" or "month". */
  every?: string;
  /** How many in all, this one included, when it repeats. */
  times?: number;
}

export interface FormErrors {
  name?: string;
  email?: string;
  phone?: string;
  notes?: string;
  seats?: string;
  /** By question text. */
  answers: Record<string, string>;
  /** Everything the page cannot put beside a field. */
  general: string[];
}

export const noErrors = (): FormErrors => ({ answers: {}, general: [] });

export const hasErrors = (e: FormErrors) => !!(e.name || e.email || e.phone || e.notes || e.seats || Object.keys(e.answers).length || e.general.length);

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** What can be seen to be wrong before asking the server (which decides all the same). */
export function check(form: Form, questions: Question[]): FormErrors {
  const e = noErrors();
  if (!form.name.trim()) e.name = t("Tell us your name");
  if (!form.email.trim()) e.email = t("We need your email to send the confirmation");
  else if (!EMAIL.test(form.email.trim())) e.email = t("That does not look like an email address");
  for (const q of questions) {
    const a = (form.answers[q.question] ?? "").trim();
    if (!a && q.required) e.answers[q.question] = t("Please answer this");
    else if (a && q.type === "Number" && Number.isNaN(Number(a))) e.answers[q.question] = t("Enter a number");
  }
  return e;
}

const LABELS = { get name() { return t("Your name"); }, get email() { return t("Email"); }, get phone() { return t("Phone number"); }, get notes() { return t("Notes"); }, get seats() { return t("Seats"); } } as const;

/** "is required" is the server's half of a sentence; the field's name is the other half. */
const sentence = (label: string, message: string) => `${label} ${message.replace(/^at most/, "can have at most")}`;

/** The server's complaints beside the fields they are about. Answers come as "Question: problem". */
export function serverErrors(error: unknown, questions: Question[]): FormErrors {
  const e = noErrors();
  if (!(error instanceof ApiError)) {
    e.general.push(error instanceof Error ? error.message : String(error));
    return e;
  }
  for (const f of error.fields) {
    const own = (Object.keys(LABELS) as (keyof typeof LABELS)[]).find((k) => k === f.field);
    const q = f.field === "answers" ? questions.find((x) => f.message.toLowerCase().startsWith(`${x.question.toLowerCase()}: `)) : undefined;
    if (own) e[own] = sentence(LABELS[own], f.message);
    else if (q) e.answers[q.question] = f.message.slice(q.question.length + 2);
    // The time, the person and the service come as whole sentences ("Asha is not available at that time").
    else e.general.push(["start_time", "staff", "service"].includes(f.field) ? f.message : `${f.field.replace(/_/g, " ")}: ${f.message}`);
  }
  if (!error.fields.length) e.general.push(error.message);
  return e;
}

/** The time went while the guest filled in the form (someone else booked it, or it was closed). */
export const isTaken = (error: unknown) => error instanceof ApiError && (error.status === 409 || error.fields.some((f) => f.field === "start_time"));

/** The request `POST /book` takes. */
export function bookBody(o: { service: Service; start: string; staff?: string; form: Form; zone: string; waitlist?: boolean; website?: string }) {
  const { service, form } = o;
  return {
    service: service.id,
    start_time: o.start,
    staff: o.staff ?? "any",
    seats: service.seats > 1 ? form.seats : 1,
    name: form.name.trim(),
    email: form.email.trim(),
    phone: form.phone.trim() || null,
    notes: form.notes.trim() || null,
    time_zone: o.zone,
    waitlist: o.waitlist || undefined,
    answers: service.questions.filter((q) => (form.answers[q.question] ?? "").trim()).map((q) => ({ question: q.question, answer: form.answers[q.question].trim() })),
    // A field people never see: only a program fills it in.
    website: o.website ?? "",
    ...(form.every && !o.waitlist && (service.repeats ?? 0) >= 2 ? { repeat_every: form.every, repeat_count: Math.min(Math.max(form.times ?? 2, 2), service.repeats ?? 2) } : {}),
  };
}

// ── the guest's calendar ──

const stamp = (ms: number) => new Date(ms).toISOString().replace(/[-:]/g, "").replace(/\.\d{3}/, "");

export interface CalendarEvent {
  title: string;
  start: number;
  end: number;
  where?: string | null;
  details?: string | null;
}

/** Links that put a booking in a web calendar (the .ics file covers the rest). */
export function calendarLinks(e: CalendarEvent): { google: string; outlook: string } {
  const q = (pairs: [string, string | null | undefined][]) =>
    pairs
      .filter(([, v]) => v)
      .map(([k, v]) => `${k}=${encodeURIComponent(v!)}`)
      .join("&");
  return {
    google: `https://calendar.google.com/calendar/render?${q([["action", "TEMPLATE"], ["text", e.title], ["dates", `${stamp(e.start)}/${stamp(e.end)}`], ["details", e.details], ["location", e.where]])}`,
    outlook: `https://outlook.live.com/calendar/0/deeplink/compose?${q([["path", "/calendar/action/compose"], ["rru", "addevent"], ["subject", e.title], ["startdt", new Date(e.start).toISOString()], ["enddt", new Date(e.end).toISOString()], ["body", e.details], ["location", e.where]])}`,
  };
}

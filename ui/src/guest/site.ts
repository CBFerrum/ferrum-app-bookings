// The booking page's endpoints (`/api/site/{workspace}/bookings/…`, the app's site handler): what a
// guest can ask for without signing in. The server holds every rule; these only carry the question.

import { api } from "@ferrum/client";
import type { Catalog, GuestBooking, Slots } from "./booking";

const base = (tenant: string) => `/api/site/${encodeURIComponent(tenant)}/bookings`;

export interface SlotQuery {
  from: string;
  to: string;
  zone: string;
  staff?: string;
  seats?: number;
}

const slotParams = (q: SlotQuery) => ({ from: q.from, to: q.to, time_zone: q.zone, staff: q.staff, seats: q.seats && q.seats > 1 ? q.seats : undefined });

export const catalog = (tenant: string, signal?: AbortSignal) => api.get<Catalog>(`${base(tenant)}/services`, undefined, signal);

/** A full class is listed too (`waitlist`), for its waitlist. */
export const freeSlots = (tenant: string, service: string, q: SlotQuery, signal?: AbortSignal) =>
  api.get<Slots>(`${base(tenant)}/slots`, { service, ...slotParams(q), waitlist: 1 }, signal);

/** A booking, or `{status: "received"}` for the hidden field only a program fills in. */
export type Booked = { booking: GuestBooking; token: string } | { status: "received" };

export const book = (tenant: string, body: unknown) => api.post<Booked>(`${base(tenant)}/book`, body);

export const mine = (tenant: string, token: string, signal?: AbortSignal) => api.get<{ booking: GuestBooking }>(`${base(tenant)}/manage/${encodeURIComponent(token)}`, undefined, signal);

/** Times a booking can move to (its own time does not block them). */
export const movableSlots = (tenant: string, token: string, q: SlotQuery, signal?: AbortSignal) =>
  api.get<Slots>(`${base(tenant)}/manage/${encodeURIComponent(token)}/slots`, slotParams(q), signal);

export const cancel = (tenant: string, token: string, reason: string) =>
  api.post<{ booking: GuestBooking }>(`${base(tenant)}/manage/${encodeURIComponent(token)}/cancel`, { reason: reason.trim() || undefined });

export const move = (tenant: string, token: string, start: string, staff?: string) =>
  api.post<{ booking: GuestBooking }>(`${base(tenant)}/manage/${encodeURIComponent(token)}/reschedule`, { start_time: start, staff: staff ?? "any" });

/** The booking as a calendar file. */
export const calendarFile = (tenant: string, token: string) => `${base(tenant)}/manage/${encodeURIComponent(token)}/booking.ics`;

// What the team's screens read from Bookings' records, and the few writes they make. The server
// holds every rule: a booking changes only through its actions (confirm, reschedule, cancel, finish),
// which answer with the booking as it now stands or say why not.

import { api, docPath } from "@ferrum/client";
import type { Bk, Hour, Off, ResourceRow, ServiceRow, StaffRow } from "./board";

export const rows = <T,>(entity: string, query: Record<string, string | number | boolean>, signal?: AbortSignal) =>
  api.get<{ rows: T[] }>(`/api/v1/${entity}`, query, signal).then((r) => r.rows);

export interface Meta {
  staff: StaffRow[];
  services: ServiceRow[];
  resources: ResourceRow[];
}

/** The people (with the hours they work), services and rooms the board is drawn from. */
export async function loadMeta(signal?: AbortSignal): Promise<Meta> {
  const [people, services, resources] = await Promise.all([
    rows<Omit<StaffRow, "hours">>("BookingStaff", { fields: "display_name,email,time_zone,active", sort: "display_name asc", limit: 200 }, signal),
    rows<ServiceRow>("BookingService", { fields: "service_name,capacity,duration_minutes,active", sort: "service_name asc", limit: 200 }, signal),
    rows<ResourceRow>("BookingResource", { fields: "resource_name,resource_type,capacity,active", sort: "resource_name asc", limit: 200 }, signal),
  ]);
  // A list leaves out a record's rows, and the weekly hours are rows.
  const staff = await Promise.all(
    people.map(async (p) => {
      const doc = await api.get<{ working_hours?: Hour[] }>(docPath("BookingStaff", p.id), undefined, signal);
      return { ...p, hours: doc.working_hours ?? [] };
    }),
  );
  return { staff, services, resources };
}

const FIELDS = "number,service,service_name,staff,staff_name,start_time,end_time,block_start,block_end,seats,status,closed_clash,resource,session_key,guest_name,source,series,amount,version";

const iso = (ms: number) => new Date(ms).toISOString();

/** The most bookings one screen asks for; a busier week says so rather than drawing half of it. */
export const LIMIT = 500;

export interface Range {
  bookings: Bk[];
  off: Off[];
}

/** Bookings that hold or held time between two instants, and the time off in that stretch. */
export async function loadRange(from: number, to: number, signal?: AbortSignal): Promise<Range> {
  const [bookings, off] = await Promise.all([
    rows<Bk>(
      "Booking",
      { filters: JSON.stringify([["start_time", "<", iso(to)], ["end_time", ">", iso(from)], ["status", "in", ["Pending", "Confirmed", "Completed", "No Show"]]]), fields: FIELDS, sort: "start_time asc", limit: LIMIT },
      signal,
    ),
    rows<Off>("StaffTimeOff", { filters: JSON.stringify([["from_time", "<", iso(to)], ["to_time", ">", iso(from)]]), fields: "staff,from_time,to_time,reason", limit: 200 }, signal),
  ]);
  return { bookings, off };
}

/** Bookings waiting for an answer or a seat, from now on. */
export const loadWaiting = (signal?: AbortSignal) =>
  rows<Bk>("Booking", { filters: JSON.stringify([["status", "in", ["Pending", "Waitlisted"]], ["end_time", ">", iso(Date.now())]]), fields: FIELDS, sort: "start_time asc", limit: 100 }, signal);

export const act = <T = Bk,>(id: string, action: string, params: Record<string, unknown> = {}) => api.post<T>(docPath("Booking", id, "actions", action), params);

export interface FreeSlot {
  start: string;
  end: string;
  seats_left: number;
  staff: { id: string; name: string }[];
}

export interface FreeDays {
  days: { date: string; slots: FreeSlot[] }[];
}

/** Times the booking's service is free, as staff may book them (a booking's own time counts as free to it). */
export const findSlots = (service: string, params: { from: string; to: string; zone: string; seats: number; booking?: string }) =>
  api.post<FreeDays>(docPath("BookingService", service, "actions", "find_slots"), { from: params.from, to: params.to, time_zone: params.zone, seats: params.seats, booking: params.booking });

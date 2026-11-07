// Where /open leads for a booking, and where the bar's Create menu leads: the schedule, on the
// booking's day with its sheet open, or with the new booking dialog.

import { useQuery } from "@tanstack/react-query";
import { Navigate, useParams } from "react-router";
import { api, docPath, t } from "@ferrum/client";
import { ErrorState, PageLoading } from "@ferrum/kit";
import { useMe } from "@ferrum/records";
import type { Bk } from "./board";
import { browserZone, dayOf } from "./zone";

export function OpenBooking() {
  const { id = "" } = useParams();
  const me = useMe();
  const q = useQuery({ queryKey: ["bk", "open", id], queryFn: ({ signal }) => api.get<Bk>(docPath("Booking", id), { titles: false }, signal) });
  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} title={t("We could not find that booking")} />;
  if (!q.data || !me.data) return <PageLoading label={t("Opening the booking…")} />;
  const day = dayOf(Date.parse(q.data.start_time), me.data.timezone || browserZone());
  return <Navigate to={`/?view=day&date=${day}&b=${id}`} replace />;
}

export function NewBookingRoute() {
  return <Navigate to="/?new=1" replace />;
}

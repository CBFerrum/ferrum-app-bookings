// A booking made by the team, from a click on an empty time of the schedule (the person and the start
// are filled in) or from the New booking button. It goes through the same engine as a guest's booking
// (hours, time off, buffers, seats, rooms), so it says why when a time will not do. The full form,
// with every field, is one click away.

import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { Link } from "react-router";
import { api, ApiError, docPath, errorMessage, t, isolate } from "@ferrum/client";
import { Alert, Button, Dialog, Field, toast } from "@ferrum/kit";
import type { Bk } from "./board";
import type { Meta } from "./data";
import { clockText, dayOf, instantAt } from "./zone";

export interface Start {
  staff?: string;
  start?: number;
  service?: string;
}

const minutesOf = (hhmm: string) => {
  const [h, m] = hhmm.split(":").map(Number);
  return Number.isFinite(h) && Number.isFinite(m) ? h * 60 + m : null;
};

export function NewBooking({ meta, zone, hour12, from, onClose, onCreated }: { meta: Meta; zone: string; hour12: boolean; from: Start; onClose: () => void; onCreated: (b: Bk) => void }) {
  const services = meta.services.filter((s) => s.active !== false);
  const [service, setService] = useState(from.service ?? (services.length === 1 ? services[0].id : ""));
  const [staff, setStaff] = useState(from.staff ?? "");
  const when = from.start ?? Math.ceil(Date.now() / 900_000) * 900_000;
  const [date, setDate] = useState(dayOf(when, zone));
  const [time, setTime] = useState(clockText(when, zone, false));
  const [seats, setSeats] = useState(1);
  const [guest, setGuest] = useState({ name: "", email: "", phone: "", notes: "" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ApiError | string | null>(null);

  const chosen = meta.services.find((s) => s.id === service);
  // Who can give the service is on the service's own record.
  const gives = useQuery({
    queryKey: ["bk", "service", service],
    enabled: !!service,
    queryFn: ({ signal }) => api.get<{ staff?: { staff: string }[] }>(docPath("BookingService", service), { titles: false }, signal),
  });
  const people = meta.staff.filter((p) => p.active !== false && (!gives.data || (gives.data.staff ?? []).some((r) => r.staff === p.id)));
  const minutes = minutesOf(time);
  const startAt = minutes === null || !date ? null : instantAt(zone, date, minutes);
  const endAt = startAt !== null && chosen?.duration_minutes ? startAt + chosen.duration_minutes * 60_000 : null;

  const fieldError = (name: string) => (error instanceof ApiError ? error.fields.find((f) => f.field === name)?.message : undefined);
  const set = (patch: Partial<typeof guest>) => setGuest((g) => ({ ...g, ...patch }));

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!service) return setError("Choose what is being booked.");
    if (startAt === null) return setError("Choose a day and a time.");
    if (!guest.name.trim()) return setError("Enter the guest's name.");
    setBusy(true);
    setError(null);
    try {
      const made = await api.post<Bk>("/api/v1/Booking", {
        service,
        staff: staff || undefined,
        start_time: new Date(startAt).toISOString(),
        seats: chosen?.capacity && chosen.capacity > 1 ? seats : 1,
        guest_name: guest.name.trim(),
        guest_email: guest.email.trim() || undefined,
        guest_phone: guest.phone.trim() || undefined,
        notes: guest.notes.trim() || undefined,
      });
      toast(t("Booked {value} for {startAt}", { value: guest.name.trim(), startAt: clockText(startAt, zone, hour12) }));
      onCreated(made);
    } catch (err) {
      setError(err instanceof ApiError ? err : errorMessage(err));
      setBusy(false);
    }
  };

  const message = error instanceof ApiError ? (error.fields.length ? error.fields.map((f) => f.message).join(". ") || error.message : error.message) : error;
  return (
    <Dialog
      open
      onClose={onClose}
      title={t("New booking")}
      footer={
        <>
          <Link className="btn btn-ghost btn-md" to={`/e/Booking/new${service ? `?service=${service}` : ""}`} onClick={onClose}>
            {t("All fields")}
          </Link>
          <span className="spacer" />
          <Button onClick={onClose}>{t("Cancel")}</Button>
          <Button variant="primary" loading={busy} type="submit" form="bk-new">
            {t("Book")}
          </Button>
        </>
      }
    >
      <form id="bk-new" className="bk-new" onSubmit={(e) => void submit(e)} noValidate>
        {message && <Alert tone="error">{isolate(message)}</Alert>}
        <Field label={t("What")} htmlFor="bk-service" required error={fieldError("service")}>
          <select id="bk-service" className="select" value={service} onChange={(e) => { setService(e.target.value); setStaff(""); }} data-autofocus={!service || undefined}>
            <option value="">{t("Choose a service…")}</option>
            {services.map((s) => (
              <option key={s.id} value={s.id}>
                {isolate(s.service_name)}
                {s.duration_minutes ? t(" · {duration_minutes} min", { duration_minutes: s.duration_minutes }) : ""}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("With")} htmlFor="bk-staff" error={fieldError("staff")}>
          <select id="bk-staff" className="select" value={staff} onChange={(e) => setStaff(e.target.value)}>
            <option value="">{t("Anyone free")}</option>
            {people.map((p) => (
              <option key={p.id} value={p.id}>
                {isolate(p.display_name)}
              </option>
            ))}
          </select>
        </Field>
        <div className="bk-new-when">
          <Field label={t("Day")} htmlFor="bk-date" required>
            <input id="bk-date" className="input" type="date" value={date} onChange={(e) => setDate(e.target.value)} />
          </Field>
          <Field label={t("Time")} htmlFor="bk-time" required error={fieldError("start_time")} hint={endAt !== null ? t("Until {endAt}", { endAt: clockText(endAt, zone, hour12) }) : undefined}>
            <input id="bk-time" className="input" type="time" step={900} value={time} onChange={(e) => setTime(e.target.value)} />
          </Field>
        </div>
        {chosen?.capacity && chosen.capacity > 1 && (
          <Field label={t("Seats")} htmlFor="bk-seats" error={fieldError("seats")} hint={t("Up to {capacity}", { capacity: chosen.capacity })}>
            <input id="bk-seats" className="input" type="number" min={1} max={chosen.capacity} value={seats} onChange={(e) => setSeats(Math.min(chosen.capacity!, Math.max(1, Number(e.target.value) || 1)))} />
          </Field>
        )}
        <Field label={t("Guest's name")} htmlFor="bk-guest" required error={fieldError("guest_name")}>
          <input id="bk-guest" className="input" autoComplete="off" maxLength={200} value={guest.name} onChange={(e) => set({ name: e.target.value })} data-autofocus={!!service || undefined} />
        </Field>
        <div className="bk-new-when">
          <Field label={t("Email")} htmlFor="bk-email" hint={t("The confirmation goes here.")} error={fieldError("guest_email")}>
            <input id="bk-email" className="input" type="email" autoComplete="off" value={guest.email} onChange={(e) => set({ email: e.target.value })} />
          </Field>
          <Field label={t("Phone")} htmlFor="bk-phone" error={fieldError("guest_phone")}>
            <input id="bk-phone" className="input" type="tel" autoComplete="off" value={guest.phone} onChange={(e) => set({ phone: e.target.value })} />
          </Field>
        </div>
        <Field label={t("Notes")} htmlFor="bk-notes">
          <textarea id="bk-notes" className="textarea" rows={2} maxLength={5000} value={guest.notes} onChange={(e) => set({ notes: e.target.value })} />
        </Field>
      </form>
    </Dialog>
  );
}

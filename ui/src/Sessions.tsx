// Class sessions scheduled before anyone books them, and the three things done to one as a whole:
// move it (every seat goes with it), repeat it into a timetable, cancel it (every guest is told). The
// server decides each one (free time, seats, rooms, who may) and the dialog shows its answer; nothing
// here is shown as done before the server has said so.

import { useQuery } from "@tanstack/react-query";
import { type FormEvent, type ReactNode, useState } from "react";
import { api, ApiError, docPath, errorMessage, t, isolate } from "@ferrum/client";
import { Alert, Button, Dialog, Field, toast } from "@ferrum/kit";
import type { Ses } from "./board";
import { actSession, type Meta } from "./data";
import { clockText, dayOf, dayText, instantAt } from "./zone";

const minutesOf = (hhmm: string) => {
  const [h, m] = hhmm.split(":").map(Number);
  return Number.isFinite(h) && Number.isFinite(m) ? h * 60 + m : null;
};

/** A refusal as a dialog shows it: beside its field when it names one, else as a sentence. */
function useRefusal() {
  const [error, setError] = useState<ApiError | string | null>(null);
  const field = (name: string) => (error instanceof ApiError ? error.fields.find((f) => f.field === name)?.message : undefined);
  const named = new Set(["service", "staff", "start_time", "capacity", "resource", "every", "count", "booking_location"]);
  const message =
    error instanceof ApiError
      ? error.status === 409
        ? t("That time is taken: {message}", { message: error.message })
        : error.fields.some((f) => named.has(f.field))
          ? null
          : error.fields.map((f) => f.message).join(". ") || error.message
      : error;
  return { field, message, fail: (e: unknown) => setError(e instanceof ApiError ? e : errorMessage(e)), clear: () => setError(null), say: setError };
}

/** Who gives a service: on the service's own record. */
function useTeachers(meta: Meta, service: string) {
  const gives = useQuery({
    queryKey: ["bk", "service", service],
    enabled: !!service,
    queryFn: ({ signal }) => api.get<{ staff?: { staff: string }[]; resources?: { resource: string }[] }>(docPath("BookingService", service), { titles: false }, signal),
  });
  return {
    people: meta.staff.filter((p) => p.active !== false && (!gives.data || (gives.data.staff ?? []).some((r) => r.staff === p.id))),
    rooms: meta.resources.filter((r) => r.active !== false && (!gives.data || (gives.data.resources ?? []).some((x) => x.resource === r.id))),
  };
}

function When({ date, time, onDate, onTime, error }: { date: string; time: string; onDate: (v: string) => void; onTime: (v: string) => void; error?: string }) {
  return (
    <div className="bk-new-when">
      <Field label={t("Day")} htmlFor="bk-ses-date" required>
        <input id="bk-ses-date" className="input" type="date" value={date} onChange={(e) => onDate(e.target.value)} />
      </Field>
      <Field label={t("Time")} htmlFor="bk-ses-time" required error={error}>
        <input id="bk-ses-time" className="input" type="time" step={900} value={time} onChange={(e) => onTime(e.target.value)} />
      </Field>
    </div>
  );
}

function Frame({ title, form, busy, label, danger, onClose, children }: { title: string; form: string; busy: boolean; label: string; danger?: boolean; onClose: () => void; children: ReactNode }) {
  return (
    <Dialog
      open
      onClose={onClose}
      title={title}
      footer={
        <>
          <span className="spacer" />
          <Button onClick={onClose}>{danger ? t("Keep it") : t("Cancel")}</Button>
          <Button variant={danger ? "danger" : "primary"} loading={busy} type="submit" form={form}>
            {label}
          </Button>
        </>
      }
    >
      {children}
    </Dialog>
  );
}

/** A new dated session of a class, before anyone books it. */
export function NewSession({ meta, zone, hour12, day, loc, onClose, onCreated }: { meta: Meta; zone: string; hour12: boolean; day: string; loc?: string | null; onClose: () => void; onCreated: (s: Ses) => void }) {
  const classes = meta.services.filter((s) => s.active !== false && (s.capacity ?? 1) > 1);
  const [service, setService] = useState(classes.length === 1 ? classes[0].id : "");
  const [staff, setStaff] = useState("");
  const [date, setDate] = useState(day);
  const [time, setTime] = useState("18:00");
  const [seats, setSeats] = useState("");
  const [room, setRoom] = useState("");
  const [place, setPlace] = useState(loc ?? "");
  const [busy, setBusy] = useState(false);
  const r = useRefusal();
  const { people, rooms } = useTeachers(meta, service);
  const chosen = classes.find((s) => s.id === service);
  const places = (meta.locations ?? []).filter((l) => l.active !== false);
  const askWhere = places.length > 1 && !meta.staff.find((p) => p.id === staff)?.location;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const minutes = minutesOf(time);
    if (!service) return r.say(t("Choose the class."));
    if (!staff) return r.say(t("Choose who teaches it."));
    if (minutes === null || !date) return r.say(t("Choose a day and a time."));
    setBusy(true);
    r.clear();
    try {
      const start = instantAt(zone, date, minutes);
      const made = await api.post<Ses>("/api/v1/BookingSession", {
        service,
        staff,
        start_time: new Date(start).toISOString(),
        capacity: seats ? Number(seats) : undefined,
        resource: room || undefined,
        booking_location: askWhere && place ? place : undefined,
      });
      toast(t("{service} scheduled for {start}", { service: chosen?.service_name ?? "", start: clockText(start, zone, hour12) }));
      onCreated(made);
    } catch (err) {
      r.fail(err);
      setBusy(false);
    }
  };

  return (
    <Frame title={t("New class session")} form="bk-ses-new" busy={busy} label={t("Schedule")} onClose={onClose}>
      <form id="bk-ses-new" className="bk-new" onSubmit={(e) => void submit(e)} noValidate>
        {r.message && <Alert tone="error">{isolate(r.message)}</Alert>}
        <Field label={t("Class")} htmlFor="bk-ses-service" required error={r.field("service")}>
          <select id="bk-ses-service" className="select" value={service} onChange={(e) => { setService(e.target.value); setStaff(""); setRoom(""); }} data-autofocus>
            <option value="">{t("Choose a class…")}</option>
            {classes.map((s) => (
              <option key={s.id} value={s.id}>
                {isolate(s.service_name)}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("Teacher")} htmlFor="bk-ses-staff" required error={r.field("staff")}>
          <select id="bk-ses-staff" className="select" value={staff} onChange={(e) => setStaff(e.target.value)}>
            <option value="">{t("Choose who teaches it…")}</option>
            {people.map((p) => (
              <option key={p.id} value={p.id}>
                {isolate(p.display_name)}
              </option>
            ))}
          </select>
        </Field>
        {askWhere && (
          <Field label={t("Location")} htmlFor="bk-ses-place" error={r.field("booking_location")}>
            <select id="bk-ses-place" className="select" value={place} onChange={(e) => setPlace(e.target.value)}>
              <option value="">{t("Choose a location…")}</option>
              {places.map((l) => (
                <option key={l.id} value={l.id}>
                  {isolate(l.location_name)}
                </option>
              ))}
            </select>
          </Field>
        )}
        <When date={date} time={time} onDate={setDate} onTime={setTime} error={r.field("start_time")} />
        <div className="bk-new-when">
          <Field label={t("Seats")} htmlFor="bk-ses-seats" error={r.field("capacity")} hint={chosen?.capacity ? t("Empty: the class's {capacity}", { capacity: chosen.capacity }) : undefined}>
            <input id="bk-ses-seats" className="input" type="number" min={1} max={1000} value={seats} onChange={(e) => setSeats(e.target.value)} />
          </Field>
          <Field label={t("Room")} htmlFor="bk-ses-room" error={r.field("resource")}>
            <select id="bk-ses-room" className="select" value={room} onChange={(e) => setRoom(e.target.value)}>
              <option value="">{t("Any free room")}</option>
              {rooms.map((x) => (
                <option key={x.id} value={x.id}>
                  {isolate(x.resource_name)}
                </option>
              ))}
            </select>
          </Field>
        </div>
      </form>
    </Frame>
  );
}

/** Another time, teacher or room for the whole session. */
export function MoveSession({ ses, meta, zone, hour12, onClose, onDone }: { ses: Ses; meta: Meta; zone: string; hour12: boolean; onClose: () => void; onDone: () => void }) {
  const start = Date.parse(ses.start_time);
  const [date, setDate] = useState(dayOf(start, zone));
  const [time, setTime] = useState(clockText(start, zone, false));
  const [staff, setStaff] = useState(ses.staff);
  const [room, setRoom] = useState(ses.resource ?? "");
  const [busy, setBusy] = useState(false);
  const r = useRefusal();
  const { people, rooms } = useTeachers(meta, ses.service);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const minutes = minutesOf(time);
    if (minutes === null || !date) return r.say(t("Choose a day and a time."));
    setBusy(true);
    r.clear();
    try {
      const to = instantAt(zone, date, minutes);
      const out = await actSession<{ moved: number }>(ses.id, "move", { start_time: new Date(to).toISOString(), staff, resource: room || undefined });
      toast(out.moved ? t("Session moved to {start}. {moved} told.", { start: clockText(to, zone, hour12), moved: out.moved }) : t("Session moved to {start}", { start: clockText(to, zone, hour12) }));
      onDone();
    } catch (err) {
      // What was typed stays, so another time can be tried.
      r.fail(err);
      setBusy(false);
    }
  };

  return (
    <Frame title={t("Move the session")} form="bk-ses-move" busy={busy} label={t("Move")} onClose={onClose}>
      <form id="bk-ses-move" className="bk-new" onSubmit={(e) => void submit(e)} noValidate>
        <p className="muted">{t("Every seat moves with it, and each guest is told.")}</p>
        {r.message && <Alert tone="error">{isolate(r.message)}</Alert>}
        <When date={date} time={time} onDate={setDate} onTime={setTime} error={r.field("start_time")} />
        <div className="bk-new-when">
          <Field label={t("Teacher")} htmlFor="bk-ses-move-staff" error={r.field("staff")}>
            <select id="bk-ses-move-staff" className="select" value={staff} onChange={(e) => setStaff(e.target.value)}>
              {people.map((p) => (
                <option key={p.id} value={p.id}>
                  {isolate(p.display_name)}
                </option>
              ))}
            </select>
          </Field>
          <Field label={t("Room")} htmlFor="bk-ses-move-room" error={r.field("resource")}>
            <select id="bk-ses-move-room" className="select" value={room} onChange={(e) => setRoom(e.target.value)}>
              <option value="">{t("The same room")}</option>
              {rooms.map((x) => (
                <option key={x.id} value={x.id}>
                  {isolate(x.resource_name)}
                </option>
              ))}
            </select>
          </Field>
        </div>
      </form>
    </Frame>
  );
}

interface Repeated {
  scheduled: { id: string; start_time: string }[];
  skipped: { date: string; start_time: string; reason: string }[];
}

/** The session made into a timetable: every week, 2 weeks, 4 weeks or month. */
export function RepeatSession({ ses, zone, hour12, onClose, onDone }: { ses: Ses; zone: string; hour12: boolean; onClose: () => void; onDone: () => void }) {
  const [every, setEvery] = useState("week");
  const [count, setCount] = useState("4");
  const [skip, setSkip] = useState(false);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<Repeated | null>(null);
  const r = useRefusal();
  const EVERY: [string, string][] = [["week", t("Every week")], ["2 weeks", t("Every 2 weeks")], ["4 weeks", t("Every 4 weeks")], ["month", t("Every month")]];

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (done) return onDone();
    setBusy(true);
    r.clear();
    try {
      const out = await actSession<Repeated>(ses.id, "repeat", { every, count: Number(count), skip_conflicts: skip ? "Yes" : "No" });
      toast(t("{n} sessions scheduled", { n: out.scheduled.length }));
      if (out.skipped.length) {
        setDone(out);
        setBusy(false);
      } else onDone();
    } catch (err) {
      r.fail(err);
      setBusy(false);
    }
  };

  return (
    <Frame title={t("Repeat the session")} form="bk-ses-repeat" busy={busy} label={done ? t("Done") : t("Repeat")} onClose={done ? onDone : onClose}>
      <form id="bk-ses-repeat" className="bk-new" onSubmit={(e) => void submit(e)} noValidate>
        {r.message && <Alert tone="error">{isolate(r.message)}</Alert>}
        {done ? (
          <>
            <Alert tone="warning">{t("These times were taken and are left out:")}</Alert>
            <ul className="bk-skipped">
              {done.skipped.map((s) => (
                <li key={s.start_time}>
                  {isolate(dayText(dayOf(Date.parse(s.start_time), zone), { weekday: "short", day: "numeric", month: "short" }))}, {isolate(clockText(Date.parse(s.start_time), zone, hour12))}: {isolate(s.reason)}
                </li>
              ))}
            </ul>
          </>
        ) : (
          <>
            <Field label={t("Every")} htmlFor="bk-ses-every" required error={r.field("every")}>
              <select id="bk-ses-every" className="select" value={every} onChange={(e) => setEvery(e.target.value)} data-autofocus>
                {EVERY.map(([v, label]) => (
                  <option key={v} value={v}>
                    {label}
                  </option>
                ))}
              </select>
            </Field>
            <Field label={t("Sessions in all")} htmlFor="bk-ses-count" required error={r.field("count")} hint={t("2 to 52, this one included")}>
              <input id="bk-ses-count" className="input" type="number" min={2} max={52} value={count} onChange={(e) => setCount(e.target.value)} />
            </Field>
            <label className="bk-check">
              <input type="checkbox" checked={skip} onChange={(e) => setSkip(e.target.checked)} /> {t("Skip the times that are taken")}
            </label>
          </>
        )}
      </form>
    </Frame>
  );
}

/** The session and every booking in it, cancelled. */
export function CancelSession({ ses, onClose, onDone }: { ses: Ses; onClose: () => void; onDone: () => void }) {
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const r = useRefusal();

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    r.clear();
    try {
      await actSession(ses.id, "cancel_session", { reason: reason.trim() || undefined });
      toast(t("Session cancelled"));
      onDone();
    } catch (err) {
      r.fail(err);
      setBusy(false);
    }
  };

  return (
    <Frame title={t("Cancel the session")} form="bk-ses-cancel" busy={busy} label={t("Cancel session")} danger onClose={onClose}>
      <form id="bk-ses-cancel" className="bk-new" onSubmit={(e) => void submit(e)} noValidate>
        <p>{t("Every booking in it is cancelled and each guest is told.")}</p>
        {r.message && <Alert tone="error">{isolate(r.message)}</Alert>}
        <Field label={t("Reason")} htmlFor="bk-ses-reason" hint={t("The guests see it.")}>
          <textarea id="bk-ses-reason" className="textarea" rows={2} maxLength={1000} value={reason} onChange={(e) => setReason(e.target.value)} data-autofocus />
        </Field>
      </form>
    </Frame>
  );
}

// Blocking out time: someone is away for an hour, a day or a week. Guests can no longer book it. Bookings
// already inside it stay (the server counts them), and the dialog says how many, so they can be moved
// or cancelled.

import { type FormEvent, useState } from "react";
import { api, ApiError, errorMessage, t, isolate } from "@ferrum/client";
import { Alert, Button, Dialog, Field, toast } from "@ferrum/kit";
import { hhmm } from "./board";
import type { Meta } from "./data";
import { addDays, instantAt } from "./zone";

export function TimeOff({ meta, zone, day, mine, onClose, onSaved }: { meta: Meta; zone: string; day: string; mine?: string; onClose: () => void; onSaved: (clashes: number) => void }) {
  const people = meta.staff.filter((p) => p.active !== false);
  const [staff, setStaff] = useState(mine ?? people[0]?.id ?? "");
  const [from, setFrom] = useState(day);
  const [to, setTo] = useState(day);
  const [allDay, setAllDay] = useState(false);
  const [starts, setStarts] = useState("12:00");
  const [ends, setEnds] = useState("13:00");
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const [a, b] = [hhmm(starts), hhmm(ends)];
    if (!staff) return setError(t("Choose who is away."));
    if (!allDay && (a === null || b === null)) return setError(t("Enter the times as hours and minutes."));
    const fromAt = instantAt(zone, from, allDay ? 0 : a!);
    // A whole day runs to the next midnight, so the last day counts.
    const toAt = allDay ? instantAt(zone, addDays(to, 1), 0) : instantAt(zone, to, b!);
    if (toAt <= fromAt) return setError(t("Time off must end after it starts."));
    setBusy(true);
    setError(null);
    try {
      const made = await api.post<{ conflicting_bookings?: number }>("/api/v1/StaffTimeOff", {
        staff,
        from_time: new Date(fromAt).toISOString(),
        to_time: new Date(toAt).toISOString(),
        reason: reason.trim() || undefined,
      });
      const clashes = made.conflicting_bookings ?? 0;
      toast(clashes ? (clashes === 1 ? t("Time blocked. {clashes} {value} inside it: move or cancel it.", { clashes, value: clashes === 1 ? t("booking falls") : t("bookings fall") }) : t("Time blocked. {clashes} {value} inside it: move or cancel them.", { clashes, value: clashes === 1 ? t("booking falls") : t("bookings fall") })) : t("Time blocked"), clashes ? "info" : "success");
      onSaved(clashes);
    } catch (err) {
      setError(err instanceof ApiError && err.fields.length ? err.fields.map((f) => f.message).join(". ") : errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      onClose={onClose}
      title={t("Block out time")}
      footer={
        <>
          <Button onClick={onClose}>{t("Cancel")}</Button>
          <Button variant="primary" type="submit" form="bk-off" loading={busy}>
            {t("Block time")}
          </Button>
        </>
      }
    >
      <form id="bk-off" className="bk-new" onSubmit={(e) => void submit(e)} noValidate>
        {error && <Alert tone="error">{isolate(error)}</Alert>}
        <Field label={t("Who is away")} htmlFor="bk-off-who" required>
          <select id="bk-off-who" className="select" value={staff} onChange={(e) => setStaff(e.target.value)} data-autofocus>
            {people.map((p) => (
              <option key={p.id} value={p.id}>
                {isolate(p.display_name)}
              </option>
            ))}
          </select>
        </Field>
        <label className="check-field">
          <input type="checkbox" checked={allDay} onChange={(e) => setAllDay(e.target.checked)} /> {t("Whole days")}
        </label>
        <div className="bk-new-when">
          <Field label={allDay ? t("First day") : t("From")} htmlFor="bk-off-from" required>
            <input id="bk-off-from" className="input" type="date" value={from} onChange={(e) => { setFrom(e.target.value); if (to < e.target.value) setTo(e.target.value); }} />
          </Field>
          {allDay ? (
            <Field label={t("Last day")} htmlFor="bk-off-to" required>
              <input id="bk-off-to" className="input" type="date" min={from} value={to} onChange={(e) => setTo(e.target.value)} />
            </Field>
          ) : (
            <Field label={t("At")} htmlFor="bk-off-start" required>
              <input id="bk-off-start" className="input" type="time" step={900} value={starts} onChange={(e) => setStarts(e.target.value)} />
            </Field>
          )}
        </div>
        {!allDay && (
          <div className="bk-new-when">
            <Field label={t("Until")} htmlFor="bk-off-to" required>
              <input id="bk-off-to" className="input" type="date" min={from} value={to} onChange={(e) => setTo(e.target.value)} />
            </Field>
            <Field label={t("At")} htmlFor="bk-off-end" required>
              <input id="bk-off-end" className="input" type="time" step={900} value={ends} onChange={(e) => setEnds(e.target.value)} />
            </Field>
          </div>
        )}
        <Field label={t("Reason")} htmlFor="bk-off-reason" hint={t("Only the team sees it.")}>
          <input id="bk-off-reason" className="input" maxLength={200} value={reason} onChange={(e) => setReason(e.target.value)} />
        </Field>
      </form>
    </Dialog>
  );
}

// Moving a booking to a time that is really free: a week of days, the times of the chosen one, and
// the person when more than one can take it. The booking's own time counts as free to it. The server
// checks the move again when it is made, and says why when it cannot.

import { useQuery } from "@tanstack/react-query";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { errorMessage, t, isolate } from "@ferrum/client";
import { Alert, Button, Spinner } from "@ferrum/kit";
import type { Bk } from "./board";
import { act, type FreeSlot, findSlots } from "./data";
import { addDays, clockText, dayOf, dayText } from "./zone";

export function Reschedule({ booking, zone, hour12, onClose, onDone }: { booking: Bk; zone: string; hour12: boolean; onClose: () => void; onDone: (b: Bk) => void }) {
  const today = dayOf(Date.now(), zone);
  const current = dayOf(Date.parse(booking.start_time), zone);
  const [from, setFrom] = useState(current > today ? current : today);
  const [day, setDay] = useState<string | null>(null);
  const [slot, setSlot] = useState<FreeSlot | null>(null);
  const [person, setPerson] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const week = useQuery({
    queryKey: ["bk", "slots", booking.id, from, zone],
    queryFn: () => findSlots(booking.service, { from, to: addDays(from, 6), zone, seats: booking.seats ?? 1, booking: booking.id }),
  });
  const days = useMemo(() => Array.from({ length: 7 }, (_, i) => addDays(from, i)), [from]);
  const free = useMemo(() => new Map((week.data?.days ?? []).filter((d) => d.slots.length).map((d) => [d.date, d.slots])), [week.data]);

  // The first day with times is chosen for them.
  useEffect(() => {
    if (week.data && (!day || !days.includes(day))) setDay(days.find((d) => free.has(d)) ?? null);
  }, [week.data, days, free, day]);

  const pick = (s: FreeSlot) => {
    setSlot(s);
    setError(null);
    // The booking stays with its person when they can take the new time.
    setPerson(s.staff.find((x) => x.id === booking.staff)?.id ?? s.staff[0]?.id ?? "");
  };

  const move = async () => {
    if (!slot) return;
    setBusy(true);
    setError(null);
    try {
      onDone(await act(booking.id, "reschedule", { start_time: slot.start, staff: person || undefined }));
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };

  const slots = day ? (free.get(day) ?? []) : [];
  return (
    <div className="bk-move">
      <div className="bk-strip-head">
        <h3>{t("Choose a new time")}</h3>
        <div className="row">
          <button type="button" className="icon-btn icon-btn-sm" aria-label={t("Earlier week")} disabled={from <= today} onClick={() => { setFrom(addDays(from, -7) < today ? today : addDays(from, -7)); setSlot(null); }}>
            <ChevronLeft size={16} />
          </button>
          <button type="button" className="icon-btn icon-btn-sm" aria-label={t("Later week")} onClick={() => { setFrom(addDays(from, 7)); setSlot(null); }}>
            <ChevronRight size={16} />
          </button>
        </div>
      </div>
      <div className="bk-strip" role="group" aria-label={t("Days")}>
        {days.map((d) => (
          <button key={d} type="button" className="bk-strip-day" aria-pressed={d === day} data-free={free.has(d) || undefined} disabled={!free.has(d)} onClick={() => { setDay(d); setSlot(null); }}>
            <span>{isolate(dayText(d, { weekday: "short" }))}</span>
            <strong>{Number(d.slice(8))}</strong>
          </button>
        ))}
      </div>
      {week.isLoading ? (
        <div className="bk-center">
          <Spinner label={t("Looking for free times")} />
        </div>
      ) : week.error ? (
        <Alert tone="error">{isolate(errorMessage(week.error))}</Alert>
      ) : !slots.length ? (
        <p className="muted">{free.size ? t("Pick a day with times.") : t("Nothing is free this week. Try a later one.")}</p>
      ) : (
        <ul className="bk-times" aria-label={t("Free times on {value}", { value: dayText(day!, { weekday: "long", day: "numeric", month: "long" }) })}>
          {slots.map((s) => (
            <li key={s.start}>
              <button type="button" className="bk-time" aria-pressed={slot?.start === s.start} onClick={() => pick(s)}>
                {isolate(clockText(Date.parse(s.start), zone, hour12))}
              </button>
            </li>
          ))}
        </ul>
      )}
      {slot && slot.staff.length > 1 && (
        <div className="field">
          <label className="field-label" htmlFor="bk-person">
            {t("With")}
          </label>
          <select id="bk-person" className="select" value={person} onChange={(e) => setPerson(e.target.value)}>
            {slot.staff.map((s) => (
              <option key={s.id} value={s.id}>
                {isolate(s.name)}
              </option>
            ))}
          </select>
        </div>
      )}
      {error && <Alert tone="error">{isolate(error)}</Alert>}
      <div className="bk-sheet-actions">
        <Button onClick={onClose}>{t("Keep it where it is")}</Button>
        <Button variant="primary" loading={busy} disabled={!slot} onClick={() => void move()}>
          {slot ? t("Move to {value}", { value: clockText(Date.parse(slot.start), zone, hour12) }) : t("Move")}
        </Button>
      </div>
    </div>
  );
}

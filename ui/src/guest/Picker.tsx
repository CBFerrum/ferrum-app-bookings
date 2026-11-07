// The month and the times of a day: the heart of the booking page, shared by a new booking and a
// move. Days that have times are open to press; the first of them is chosen for the guest, and when
// a month has none we look ahead rather than show an empty calendar. The list of times refreshes when
// the guest comes back to the tab and every few minutes, so what is offered is what is free.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import { EmptyState, ErrorState, Icon } from "@ferrum/kit";
import { addDays, addMonths, clockText, dayText, firstWeekday, monthEnd, monthOf, monthWeeks } from "../zone";
import { firstDay, openDays, type Slot, type Slots } from "./booking";
import { t, isolate } from "@ferrum/client";

/** How many months ahead we look for the first day with times. */
const LOOK_AHEAD = 6;

export function MonthGrid({ month, first, open, selected, today, busy, canPrev, onSelect, onMonth }: {
  month: string;
  first: number;
  open: Map<string, Slot[]>;
  selected: string | null;
  today: string;
  busy: boolean;
  canPrev: boolean;
  onSelect: (date: string) => void;
  onMonth: (month: string) => void;
}) {
  const weeks = useMemo(() => monthWeeks(month, first), [month, first]);
  const cells = useRef(new Map<string, HTMLButtonElement>());
  const focusAfter = useRef<string | null>(null);
  // The one day Tab stops on: the chosen day, else the first with times, else the first of the month.
  const stop = selected && monthOf(selected) === month ? selected : ([...open.keys()].sort().find((d) => monthOf(d) === month) ?? `${month}-01`);
  const title = dayText(`${month}-01`, { month: "long", year: "numeric" });

  useEffect(() => {
    if (focusAfter.current) cells.current.get(focusAfter.current)?.focus();
    focusAfter.current = null;
  }, [month]);

  const onKeyDown = (e: KeyboardEvent<HTMLTableElement>) => {
    const from = (document.activeElement as HTMLElement | null)?.dataset.date;
    const by: Record<string, number> = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 };
    if (!from || !(e.key in by || e.key === "PageUp" || e.key === "PageDown" || e.key === "Home" || e.key === "End")) return;
    e.preventDefault();
    const weekStartOf = (d: string) => addDays(d, -weeks.flat().indexOf(d) % 7);
    const to =
      e.key === "PageUp" ? `${addMonths(monthOf(from), -1)}-01`
      : e.key === "PageDown" ? `${addMonths(monthOf(from), 1)}-01`
      : e.key === "Home" ? weekStartOf(from)
      : e.key === "End" ? addDays(weekStartOf(from), 6)
      : addDays(from, by[e.key]);
    if (monthOf(to) === month) cells.current.get(to)?.focus();
    else if (!(monthOf(to) < monthOf(today))) {
      focusAfter.current = to;
      onMonth(monthOf(to));
    }
  };

  return (
    <div className="gb-cal" aria-busy={busy}>
      <div className="gb-cal-head">
        <h2 className="gb-cal-title" aria-live="polite">
          {isolate(title)}
        </h2>
        <div className="gb-cal-nav">
          <button type="button" className="icon-btn" aria-label={t("Previous month")} disabled={!canPrev} onClick={() => onMonth(addMonths(month, -1))}>
            <Icon name="left" size={18} />
          </button>
          <button type="button" className="icon-btn" aria-label={t("Next month")} onClick={() => onMonth(addMonths(month, 1))}>
            <Icon name="chevronRight" size={18} />
          </button>
        </div>
      </div>
      <table className="gb-grid" role="grid" aria-label={title} onKeyDown={onKeyDown}>
        <thead>
          <tr>
            {weeks[0].map((d) => (
              <th key={d} scope="col" abbr={dayText(d, { weekday: "long" })}>
                {isolate(dayText(d, { weekday: "short" }))}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {weeks.map((week) => (
            <tr key={week[0]}>
              {week.map((d) => {
                if (monthOf(d) !== month) return <td key={d} aria-hidden="true" />;
                const times = open.get(d)?.length ?? 0;
                const label = `${dayText(d, { weekday: "long", day: "numeric", month: "long" })}, ${times ? (times === 1 ? t("1 time free") : t("{times} times free", { times })) : "nothing free"}`;
                return (
                  <td key={d} role="gridcell" aria-selected={d === selected}>
                    <button
                      type="button"
                      ref={(el) => void (el ? cells.current.set(d, el) : cells.current.delete(d))}
                      data-date={d}
                      data-open={times > 0 || undefined}
                      data-today={d === today || undefined}
                      data-selected={d === selected || undefined}
                      aria-label={label}
                      aria-disabled={!times || undefined}
                      tabIndex={d === stop ? 0 : -1}
                      onClick={() => times && onSelect(d)}
                    >
                      {Number(d.slice(8))}
                    </button>
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function Times({ day, slots, zone, hour12, at, showSeats, onPick }: {
  day: string;
  slots: Slot[];
  zone: string;
  hour12: boolean;
  at: string | null;
  showSeats: boolean;
  onPick: (s: Slot) => void;
}) {
  const heading = dayText(day, { weekday: "long", day: "numeric", month: "long" });
  return (
    <div className="gb-times">
      <h3 className="gb-times-title" id="gb-times-title">
        {isolate(heading)}
      </h3>
      <ul className="gb-slots" aria-labelledby="gb-times-title">
        {slots.map((s) => (
          <li key={s.start}>
            <button
              type="button"
              className="gb-slot"
              data-full={s.full || undefined}
              aria-pressed={s.start === at}
              aria-label={s.full ? t("{value}, full, join the waitlist", { value: clockText(Date.parse(s.start), zone, hour12) }) : undefined}
              onClick={() => onPick(s)}
            >
              {isolate(clockText(Date.parse(s.start), zone, hour12))}
              {s.full ? <small>{t("Full · waitlist")}</small> : showSeats && <small>{t("{seats_left} left", { seats_left: s.seats_left })}</small>}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

export interface PickerProps {
  /** What is being asked for (service, person, seats, zone): a change starts the search afresh. */
  scope: unknown[];
  load: (from: string, to: string, signal: AbortSignal) => Promise<Slots>;
  zone: string;
  hour12: boolean;
  today: string;
  day: string | null;
  at: string | null;
  /** Seats of a class are shown beside its times. */
  showSeats: boolean;
  onDay: (date: string) => void;
  onPick: (s: Slot) => void;
}

export function Picker(p: PickerProps) {
  const qc = useQueryClient();
  const first = useMemo(firstWeekday, []);
  const [month, setMonth] = useState(() => monthOf(p.day ?? p.today));
  const [hops, setHops] = useState(0);
  const [steered, setSteered] = useState(false);
  const times = useRef<HTMLDivElement>(null);

  const fetchMonth = (m: string) => ({
    queryKey: ["gb-slots", ...p.scope, m],
    queryFn: ({ signal }: { signal: AbortSignal }) => p.load(m === monthOf(p.today) ? p.today : `${m}-01`, monthEnd(m), signal),
    staleTime: 30_000,
  });
  const q = useQuery({ ...fetchMonth(month), refetchInterval: 5 * 60_000, refetchOnWindowFocus: true });
  const open = useMemo(() => openDays(q.data), [q.data]);

  // A change of service, person or zone starts again from the first month.
  const scopeKey = JSON.stringify(p.scope);
  const seen = useRef(scopeKey);
  useEffect(() => {
    if (seen.current === scopeKey) return;
    seen.current = scopeKey;
    setMonth(monthOf(p.today));
    setHops(0);
    setSteered(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scopeKey]);

  // Choose the first day with times for the guest, looking ahead when this month has none.
  useEffect(() => {
    if (!q.data || q.isFetching || p.day) return;
    const day = firstDay(open);
    if (day) p.onDay(day);
    else if (!steered && hops < LOOK_AHEAD) {
      setHops((h) => h + 1);
      setMonth((m) => addMonths(m, 1));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [q.data, q.isFetching, p.day]);

  // The next month is fetched before it is asked for.
  useEffect(() => {
    if (q.data) void qc.prefetchQuery(fetchMonth(addMonths(month, 1)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [q.data, month]);

  const goTo = (m: string) => {
    setSteered(true);
    setMonth(m);
  };
  const nothingAhead = !!q.data && !p.day && !open.size && (steered || hops >= LOOK_AHEAD);
  // The chosen day's times, while its month is the one on show.
  const here = p.day && monthOf(p.day) === month ? p.day : null;
  const slots = here ? open.get(here) : undefined;

  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} title={t("We could not load the times")} />;
  return (
    <div className="gb-picker">
      <MonthGrid
        month={month}
        first={first}
        open={open}
        selected={p.day}
        today={p.today}
        busy={q.isFetching && !q.data}
        canPrev={month > monthOf(p.today)}
        onSelect={(d) => {
          p.onDay(d);
          // On a phone the times are below the month.
          if (window.matchMedia?.("(max-width: 880px)").matches) times.current?.scrollIntoView({ behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth", block: "start" });
        }}
        onMonth={goTo}
      />
      <div ref={times} className="gb-times-wrap">
        <p className="sr-only" role="status">
          {here && slots ? t("{length} {value} on {here}", { length: slots.length, value: slots.length === 1 ? "time" : "times", here: dayText(here, { weekday: "long", day: "numeric", month: "long" }) }) : ""}
        </p>
        {!q.data ? (
          <div className="gb-slots-loading" aria-label={t("Loading times")} role="status">
            {[0, 1, 2, 3, 4, 5].map((i) => (
              <span key={i} className="skeleton" />
            ))}
          </div>
        ) : here && slots ? (
          <Times day={here} slots={slots} zone={p.zone} hour12={p.hour12} at={p.at} showSeats={p.showSeats} onPick={p.onPick} />
        ) : nothingAhead ? (
          <EmptyState title={t("No times free right now")} icon="calendar">
            <p className="muted">{t("There is nothing open in the coming months. Try again soon, or get in touch with us.")}</p>
          </EmptyState>
        ) : here ? (
          <EmptyState title={t("Nothing free that day")} icon="calendar">
            <p className="muted">{t("Choose another day. Days with times are in bold.")}</p>
          </EmptyState>
        ) : (
          <p className="muted gb-looking">{open.size ? t("Pick a day to see its times.") : t("Looking for the first free day…")}</p>
        )}
      </div>
    </div>
  );
}

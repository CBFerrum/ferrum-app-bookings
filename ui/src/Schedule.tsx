// The schedule: who is where, a day or a week at a time, as the front desk reads it. One column per
// person (or room, or day), the bookings that need an answer in a strip above, a click on an empty
// time to book it, a drag (or M and the arrow keys) to move a booking, and a sheet for the booking
// itself. Day, date and filters live in the address, so a link shows what the sender saw. A move shows
// at once and goes back, with the server's reason, if it is refused.

import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { CalendarOff, ChevronLeft, ChevronRight, Plus } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { errorMessage, t as tr, isolate } from "@ferrum/client";
import { Alert, Badge, Button, StatePanel, toast, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { can, useEntityMeta, useMe } from "@ferrum/records";
import { Board } from "./Grid";
import { type Bk, blocks as makeBlocks, type Block, dropHint, type Lane, makeLane, tone, viewWindow } from "./board";
import { act, LIMIT, loadMeta, loadRange, loadWaiting, type Range } from "./data";
import { NewBooking, type Start } from "./NewBooking";
import { Pop } from "./Pop";
import { BookingSheet, refreshBookings, SessionSheet } from "./Sheet";
import { Failed, Loading, Nothing } from "./states";
import { Tabs } from "./Tabs";
import { TimeOff } from "./TimeOff";
import { addDays, browserZone, clockText, dayBounds, dayOf, dayText, firstWeekday, isDate, prefers12Hour, weekStart } from "./zone";

const NONE = "none";

/** What waits for the team: pending bookings to confirm or decline, and people waiting for a seat. */
function Approvals({ waiting, zone, hour12, canWrite, onOpen, onDone }: { waiting: Bk[]; zone: string; hour12: boolean; canWrite: boolean; onOpen: (b: Bk) => void; onDone: () => void }) {
  // On a phone the strip starts folded, with its count in view, so the schedule is not pushed down the screen.
  const [open, setOpen] = useState(() => !globalThis.matchMedia?.("(max-width: 720px)").matches);
  const [busy, setBusy] = useState<string | null>(null);
  const pending = waiting.filter((b) => b.status === "Pending");
  const queued = waiting.filter((b) => b.status === "Waitlisted");
  if (!waiting.length) return null;

  const confirm = async (b: Bk) => {
    setBusy(b.id);
    try {
      await act(b.id, "confirm");
      toast(tr("Confirmed {value}. They are told.", { value: b.guest_name ?? tr("the booking") }));
      onDone();
    } catch (e) {
      toast(errorMessage(e), "error");
    } finally {
      setBusy(null);
    }
  };

  const card = (b: Bk) => {
    const start = Date.parse(b.start_time);
    return (
      <li key={b.id} className="bk-ask">
        <button type="button" className="bk-ask-main" onClick={() => onOpen(b)}>
          <strong>{b.guest_name ?? tr("Guest")}</strong>
          <span>
            {isolate(b.service_name ?? "")}
            {b.staff_name ? ` · ${b.staff_name}` : ""}
          </span>
          <span className="faint">
            {isolate(dayText(dayOf(start, zone), { weekday: "short", day: "numeric", month: "short" }))}, {isolate(clockText(start, zone, hour12))}
          </span>
        </button>
        {canWrite && b.status === "Pending" && (
          <div className="bk-ask-actions">
            <Button size="sm" variant="primary" loading={busy === b.id} onClick={() => void confirm(b)}>
              {tr("Confirm")}
            </Button>
            <Button size="sm" onClick={() => onOpen(b)}>
              {tr("Decline…")}
            </Button>
          </div>
        )}
        {b.status === "Waitlisted" && <Badge tone={tone(b.status)}>{tr("Waiting for a seat")}</Badge>}
      </li>
    );
  };

  return (
    <section className="bk-approvals" aria-label={tr("Needs your answer")}>
      <header>
        <h2>
          {pending.length ? tr("{length} to confirm", { length: pending.length }) : tr("No one to confirm")}
          {queued.length > 0 && <span className="faint"> {tr("· {length} waiting for a seat", { length: queued.length })}</span>}
        </h2>
        <button type="button" className="link-btn" aria-expanded={open} onClick={() => setOpen(!open)}>
          {open ? tr("Hide") : tr("Show")}
        </button>
      </header>
      {open && <ul className="bk-asks">{[...pending, ...queued].map(card)}</ul>}
    </section>
  );
}

export function Schedule() {
  useShellTitle(tr("Schedule"));
  const qc = useQueryClient();
  const me = useMe().data;
  const zone = me?.timezone || browserZone();
  const hour12 = useMemo(prefers12Hour, []);
  const first = useMemo(firstWeekday, []);
  const [sp, setSp] = useSearchParams();
  const [draft, setDraft] = useState<Start | null>(() => (sp.has("new") ? {} : null));
  const [blocking, setBlocking] = useState(false);
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(t);
  }, []);

  const today = dayOf(now, zone);
  const view = sp.get("view") === "week" ? "week" : "day";
  const by = sp.get("by") === "room" && view === "day" ? "room" : "staff";
  const dateParam = sp.get("date");
  const date = isDate(dateParam) ? dateParam : today;
  const chosen = (sp.get("staff") ?? "").split(",").filter(Boolean);
  const service = sp.get("service") ?? "";
  const openId = sp.get("b");
  const openSession = sp.get("s");

  const set = useCallback(
    (patch: Record<string, string | null>, replace = false) => {
      setSp(
        (prev) => {
          const next = new URLSearchParams(prev);
          for (const [k, v] of Object.entries(patch)) {
            if (v === null || v === "") next.delete(k);
            else next.set(k, v);
          }
          return next;
        },
        { replace },
      );
    },
    [setSp],
  );

  const days = view === "week" ? Array.from({ length: 7 }, (_, i) => addDays(weekStart(date, first), i)) : [date];
  const from = dayBounds(zone, days[0])[0];
  const to = dayBounds(zone, days[days.length - 1])[1];

  const meta = useQuery({ queryKey: ["bk", "meta"], queryFn: ({ signal }) => loadMeta(signal), staleTime: 5 * 60_000 });
  const range = useQuery({ queryKey: ["bk", "range", from, to], queryFn: ({ signal }) => loadRange(from, to, signal), placeholderData: keepPreviousData });
  const waiting = useQuery({ queryKey: ["bk", "waiting"], queryFn: ({ signal }) => loadWaiting(signal) });
  const bookingMeta = useEntityMeta("Booking");
  const canWrite = !!bookingMeta.data && can(bookingMeta.data.permissions, "write");
  const canCreate = !!bookingMeta.data && can(bookingMeta.data.permissions, "create");
  const offMeta = useEntityMeta("StaffTimeOff");
  const canBlock = !!offMeta.data && can(offMeta.data.permissions, "create");

  const refresh = useCallback(() => refreshBookings(qc), [qc]);
  useLiveEntity("Booking", refresh);
  useLiveEntity("StaffTimeOff", useCallback(() => void qc.invalidateQueries({ queryKey: ["bk", "range"] }), [qc]));

  const chosenStaff = meta.data?.staff.find((p) => p.id === chosen[0]);
  const services = useMemo(() => new Map((meta.data?.services ?? []).map((s) => [s.id, s])), [meta.data]);
  const bookings = useMemo(() => (range.data?.bookings ?? []).filter((b) => !service || b.service === service), [range.data, service]);
  const all = useMemo(() => makeBlocks(bookings, services), [bookings, services]);

  // Who has a column: the people picked (everyone active when none is), and anyone with a booking in view.
  const people = useMemo(() => {
    const staff = meta.data?.staff ?? [];
    const busy = new Set(all.map((b) => b.staff));
    const shown = staff.filter((p) => (p.active !== false || busy.has(p.id)) && (!chosen.length || chosen.includes(p.id)));
    return shown;
  }, [meta.data, all, chosen.join(",")]);
  const everyone = useMemo(() => (meta.data?.staff ?? []).filter((p) => p.active !== false), [meta.data]);
  const resources = useMemo(() => (meta.data?.resources ?? []).filter((r) => r.active !== false), [meta.data]);

  const off = useMemo(() => range.data?.off ?? [], [range.data]);
  const lanes: Lane[] = useMemo(() => {
    if (view === "week") {
      const who = chosen.length === 1 ? everyone.filter((p) => p.id === chosen[0]) : everyone;
      return days.map((d) => makeLane({ id: d, label: dayText(d, { weekday: "short" }), sub: dayText(d, { day: "numeric", month: "short" }), date: d, zone, people: who, offs: off }));
    }
    if (by === "room") {
      const rooms = [...resources.map((r) => ({ id: r.id, label: r.resource_name, sub: r.resource_type ?? undefined })), { id: NONE, label: tr("No room"), sub: undefined }];
      return rooms.map((r) => makeLane({ id: r.id, label: r.label, sub: r.sub, date, zone, people: everyone, offs: [] }));
    }
    return people.map((p) => makeLane({ id: p.id, label: p.display_name, date, zone, people: [p], offs: off }));
  }, [view, by, days.join(","), people, everyone, resources, off, zone, date, chosen.join(",")]);

  const placed = useMemo(() => {
    // Week: the person picked, when one is.
    const mine = view === "week" && chosen.length === 1 ? all.filter((b) => b.staff === chosen[0]) : all;
    return mine;
  }, [all, view, chosen.join(",")]);
  const win = useMemo(() => viewWindow(lanes, placed), [lanes, placed]);

  const laneOf = useCallback(
    (b: Block) => (view === "week" ? lanes.find((l) => b.start >= l.start && b.start < l.end)?.id : by === "room" ? (b.resource ?? NONE) : (b.staff ?? undefined)),
    [view, by, lanes],
  );
  const staffName = useMemo(() => new Map((meta.data?.staff ?? []).map((p) => [p.id, p.display_name])), [meta.data]);
  const initialsOf = useCallback((id: string | null) => (id ? (staffName.get(id) ?? "").split(/\s+/).map((w) => w[0]).join("").slice(0, 2).toUpperCase() : ""), [staffName]);

  // A person's own column knows their hours and time off; others only the clashes.
  const limited = view === "day" && by === "staff";
  const hintFor = useCallback(
    (b: Block, lane: Lane, start: number) => {
      const who = limited ? lane.id : b.staff;
      return dropHint(b, start, all.filter((x) => x.staff === who), lane.work, lane.off, limited || (view === "week" && chosen.length === 1));
    },
    [all, limited, view, chosen.join(",")],
  );

  const move = async (block: Block, lane: Lane, start: number) => {
    const b = block.bookings[0];
    const shift = start - block.start;
    const staff = limited && lane.id !== b.staff ? lane.id : undefined;
    const slide = (t?: string | null) => (t ? new Date(Date.parse(t) + shift).toISOString() : t);
    const before = qc.getQueriesData<Range>({ queryKey: ["bk", "range"] });
    // It moves at once; the server may still say no.
    qc.setQueriesData<Range>({ queryKey: ["bk", "range"] }, (old) =>
      old && {
        ...old,
        bookings: old.bookings.map((x) => (x.id === b.id ? { ...x, start_time: slide(x.start_time)!, end_time: slide(x.end_time)!, block_start: slide(x.block_start), block_end: slide(x.block_end), ...(staff ? { staff, staff_name: staffName.get(staff) ?? x.staff_name } : {}) } : x)),
      },
    );
    try {
      const saved = await act(b.id, "reschedule", { start_time: new Date(start).toISOString(), ...(staff ? { staff } : {}) });
      qc.setQueriesData<Range>({ queryKey: ["bk", "range"] }, (old) => old && { ...old, bookings: old.bookings.map((x) => (x.id === b.id ? { ...x, ...saved } : x)) });
      toast(tr("{value} moved to {start}", { value: b.guest_name ?? tr("The booking"), start: clockText(start, zone, hour12) }));
      void qc.invalidateQueries({ queryKey: ["bk", "doc", b.id] });
    } catch (e) {
      for (const [key, data] of before) qc.setQueryData(key, data);
      toast(errorMessage(e), "error");
      refresh();
    }
  };

  const step = (n: number) => set({ date: addDays(date, view === "week" ? n * 7 : n) }, true);
  const openBlock = (b: Block) => set(b.kind === "booking" ? { b: b.bookings[0].id, s: null } : { s: b.key, b: null });
  const closeSheet = () => {
    const key = openId ?? openSession;
    set({ b: null, s: null }, true);
    // The focus goes back to what opened it.
    if (key) requestAnimationFrame(() => document.getElementById(`bk-${placed.find((x) => x.bookings.some((y) => y.id === key) || x.key === key)?.key}`)?.focus());
  };
  const session = openSession ? all.find((b) => b.key === openSession) : undefined;

  const onLane = (lane: Lane) => {
    if (view === "week") set({ view: "day", date: lane.date });
    else if (by === "staff") set({ view: "week", staff: lane.id });
  };
  const count = (lane: Lane) => {
    const n = placed.filter((b) => laneOf(b) === lane.id).length;
    return n === 0 ? tr("Nothing booked") : tr("{n} booked", { n });
  };

  const heading = view === "week" ? `${dayText(days[0], { day: "numeric", month: "short" })} – ${dayText(days[6], { day: "numeric", month: "short", year: "numeric" })}` : dayText(date, { weekday: "long", day: "numeric", month: "long", year: "numeric" });
  const selected = openId ? (all.find((b) => b.bookings.some((x) => x.id === openId))?.key ?? null) : openSession;
  const pendingCount = (waiting.data ?? []).filter((b) => b.status === "Pending").length;

  let body;
  if (meta.error) body = <Failed error={meta.error} onRetry={() => void meta.refetch()} title={tr("We could not load the schedule")} />;
  else if (!meta.data) body = <Loading title={tr("Loading the schedule…")} />;
  else if (!meta.data.staff.length) {
    body = (
      <Nothing
        title={tr("No one to book yet")}
        action={
          <Link className="btn btn-primary btn-md" to="/e/BookingStaff/new">
            {tr("Add a staff member")}
          </Link>
        }
      >
        {tr("Add the people guests book, with the hours they work, and they appear here.")}
      </Nothing>
    );
  } else if (range.error && !range.data) body = <Failed error={range.error} onRetry={() => void range.refetch()} title={tr("We could not load the bookings")} />;
  else if (!lanes.length) {
    body = (
      by === "staff" && chosen.length ? (
        <StatePanel kind="no-matches" title={tr("No one is selected")} onClear={() => set({ staff: null })} clearLabel={tr("Show everyone")}>
          {tr("Choose who to show in People.")}
        </StatePanel>
      ) : (
        <Nothing title={by === "room" ? tr("No rooms yet") : tr("No one is selected")}>
          {by === "room" ? tr("Rooms and equipment that services need are set up in Resources.") : tr("Choose who to show in People.")}
        </Nothing>
      )
    );
  } else {
    body = (
      <div className="bk-stage" data-busy={range.isFetching || undefined}>
        {range.data && range.data.bookings.length >= LIMIT && <Alert tone="warning">{tr("Only the first {LIMIT} bookings of this {view} are shown. Choose fewer days or one person.", { LIMIT, view })}</Alert>}
        <Board
          lanes={lanes}
          blocks={placed}
          laneOf={laneOf}
          win={win}
          zone={zone}
          hour12={hour12}
          now={now}
          selected={selected}
          canMove={canWrite}
          acrossLanes={view === "week" || by === "staff"}
          canCreate={canCreate}
          showWho={view === "week" || by === "room"}
          whoInitials={initialsOf}
          hintFor={hintFor}
          note={count}
          onOpen={openBlock}
          onCreate={(lane, start) => setDraft({ start, staff: limited ? lane.id : view === "week" && chosen.length === 1 ? chosen[0] : undefined })}
          onMove={(b, l, s) => void move(b, l, s)}
          onLane={onLane}
        />
        {range.data && !placed.length && (
          <p className="bk-empty muted">
            {view === "week" ? tr("Nothing is booked this week.") : date === today ? tr("Nothing is booked today.") : tr("Nothing is booked this day.")}
            {canCreate ? tr(" Press a time to book it.") : ""}
          </p>
        )}
        <p className="bk-hint faint">{(canWrite ? tr("Drag a booking to move it, or select it and press M, then the arrow keys. Shaded time is outside working hours; hatched is time off.") : tr("Select a booking to see it. Shaded time is outside working hours; hatched is time off."))}</p>
      </div>
    );
  }

  return (
    <div className="bk-page">
      <header className="bk-head">
        <div>
          <Tabs />
          <p className="page-sub" aria-live="polite">
            {range.data ? `${placed.length} ${placed.length === 1 ? "booking" : "bookings"}${pendingCount ? tr(" · {pendingCount} to confirm", { pendingCount }) : ""}` : " "}
          </p>
        </div>
        <div className="row">
          {canBlock && meta.data?.staff.length ? (
            <Button onClick={() => setBlocking(true)}>
              <CalendarOff size={16} aria-hidden="true" /> {tr("Block time")}
            </Button>
          ) : null}
          {canCreate && meta.data?.staff.length ? (
            <Button variant="primary" onClick={() => setDraft({})}>
              <Plus size={16} aria-hidden="true" /> {tr("New booking")}
            </Button>
          ) : null}
        </div>
      </header>

      <div className="bk-tools" role="toolbar" aria-label={tr("Schedule")}>
        <div className="bk-nav">
          <button type="button" className="icon-btn" aria-label={view === "week" ? tr("Earlier week") : tr("Earlier day")} onClick={() => step(-1)}>
            <ChevronLeft size={18} />
          </button>
          <Button size="sm" onClick={() => set({ date: null }, true)} disabled={days.includes(today)}>
            {tr("Today")}
          </Button>
          <button type="button" className="icon-btn" aria-label={view === "week" ? tr("Later week") : tr("Later day")} onClick={() => step(1)}>
            <ChevronRight size={18} />
          </button>
          <h2 className="bk-date">{isolate(heading)}</h2>
          <input className="input bk-date-input" type="date" aria-label={tr("Go to a date")} value={date} onChange={(e) => isDate(e.target.value) && set({ date: e.target.value }, true)} />
        </div>
        <div className="bk-filters">
          <div className="segmented" role="group" aria-label={tr("View")}>
            <button type="button" aria-pressed={view === "day"} onClick={() => set({ view: null, by: by === "room" ? "room" : null })}>
              {tr("Day")}
            </button>
            <button type="button" aria-pressed={view === "week"} onClick={() => set({ view: "week", by: null, staff: chosen.slice(0, 1).join(",") || null })}>
              {tr("Week")}
            </button>
          </div>
          {view === "day" && resources.length > 0 && (
            <div className="segmented" role="group" aria-label={tr("Columns")}>
              <button type="button" aria-pressed={by === "staff"} onClick={() => set({ by: null })}>
                {tr("People")}
              </button>
              <button type="button" aria-pressed={by === "room"} onClick={() => set({ by: "room" })}>
                {tr("Rooms")}
              </button>
            </div>
          )}
          {meta.data && by === "staff" && view === "day" && meta.data.staff.length > 1 && (
            <Pop summary={chosen.length ? tr("{length} of {length2} people", { length: chosen.length, length2: everyone.length }) : tr("Everyone")}>
              <fieldset className="bk-checks">
                <legend className="sr-only">{tr("People to show")}</legend>
                {everyone.map((p) => (
                  <label key={p.id}>
                    <input
                      type="checkbox"
                      checked={!chosen.length || chosen.includes(p.id)}
                      onChange={(e) => {
                        const now = chosen.length ? chosen : everyone.map((x) => x.id);
                        const next = e.target.checked ? [...now, p.id] : now.filter((x) => x !== p.id);
                        set({ staff: next.length === everyone.length || !next.length ? null : next.join(",") }, true);
                      }}
                    />
                    {isolate(p.display_name)}
                  </label>
                ))}
                {chosen.length > 0 && (
                  <button type="button" className="link-btn" onClick={() => set({ staff: null }, true)}>
                    {tr("Show everyone")}
                  </button>
                )}
              </fieldset>
            </Pop>
          )}
          {meta.data && view === "week" && meta.data.staff.length > 1 && (
            <select className="select bk-select" aria-label={tr("Whose week")} value={chosen[0] ?? ""} onChange={(e) => set({ staff: e.target.value || null }, true)}>
              <option value="">{tr("Everyone")}</option>
              {everyone.map((p) => (
                <option key={p.id} value={p.id}>
                  {isolate(p.display_name)}
                </option>
              ))}
            </select>
          )}
          {meta.data && meta.data.services.length > 1 && (
            <select className="select bk-select" aria-label={tr("Service")} value={service} onChange={(e) => set({ service: e.target.value || null }, true)}>
              <option value="">{tr("All services")}</option>
              {meta.data.services.map((s) => (
                <option key={s.id} value={s.id}>
                  {isolate(s.service_name)}
                </option>
              ))}
            </select>
          )}
        </div>
      </div>

      <Approvals
        waiting={waiting.data ?? []}
        zone={zone}
        hour12={hour12}
        canWrite={canWrite}
        onOpen={(b) => set({ b: b.id, s: null, date: dayOf(Date.parse(b.start_time), zone), view: "day" })}
        onDone={refresh}
      />

      <div className="bk-layout" data-sheet={openId || session ? "open" : undefined}>
        <div className="bk-main">{body}</div>
        {openId ? (
          <BookingSheet key={openId} id={openId} zone={zone} hour12={hour12} canWrite={canWrite} onClose={closeSheet} />
        ) : session ? (
          <SessionSheet bookings={session.bookings} zone={zone} hour12={hour12} onOpen={(id) => set({ b: id, s: null })} onClose={closeSheet} />
        ) : null}
      </div>

      {blocking && meta.data && (
        <TimeOff
          meta={meta.data}
          zone={zone}
          day={date}
          mine={(meta.data.staff.find((p) => p.email.toLowerCase() === me?.user.email.toLowerCase()) ?? chosenStaff)?.id}
          onClose={() => setBlocking(false)}
          onSaved={() => {
            setBlocking(false);
            void qc.invalidateQueries({ queryKey: ["bk", "range"] });
          }}
        />
      )}

      {draft && meta.data && (
        <NewBooking
          meta={meta.data}
          zone={zone}
          hour12={hour12}
          from={draft}
          onClose={() => {
            setDraft(null);
            if (sp.has("new")) set({ new: null }, true);
          }}
          onCreated={(b) => {
            setDraft(null);
            refresh();
            set({ new: null, date: dayOf(Date.parse(b.start_time), zone), view: view === "week" ? "week" : "day", b: b.id, s: null }, true);
          }}
        />
      )}
    </div>
  );
}

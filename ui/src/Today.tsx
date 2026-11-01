// Today, in the order it will happen: what needs an answer, what is under way, what comes next, and
// what has happened and still needs closing (did they come?). The person's own bookings by default,
// everyone's for those who run the calendar. The buttons on a row are the ones the server allows for
// its status; a class is one row with its seats beneath.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { errorMessage } from "@ferrum/client";
import { Badge, Button, EmptyState, ErrorState, PageLoading, toast, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { can, useEntityMeta, useMe } from "@ferrum/records";
import { type Bk, blocks as makeBlocks, type Block, inOrder, nextSteps, tone } from "./board";
import { act, loadMeta, loadRange, loadWaiting } from "./data";
import { BookingSheet, refreshBookings } from "./Sheet";
import { Tabs } from "./Tabs";
import { browserZone, clockText, dayBounds, dayOf, dayText, prefers12Hour } from "./zone";

type Do = (b: Bk, action: "confirm" | "complete" | "mark_no_show") => Promise<void>;

/** The buttons a booking offers right here: the quick ones, not the move and the cancel. */
function Quick({ b, now, canWrite, busy, run }: { b: Bk; now: number; canWrite: boolean; busy: string | null; run: Do }) {
  const steps = canWrite ? nextSteps(b, now) : [];
  const ended = Date.parse(b.end_time) <= now;
  return (
    <div className="bk-quick">
      {steps.includes("confirm") && (
        <Button size="sm" variant="primary" loading={busy === `${b.id}confirm`} onClick={() => void run(b, "confirm")}>
          Confirm
        </Button>
      )}
      {steps.includes("complete") && (
        <Button size="sm" variant={ended ? "primary" : "default"} loading={busy === `${b.id}complete`} onClick={() => void run(b, "complete")}>
          Came
        </Button>
      )}
      {steps.includes("mark_no_show") && (
        <Button size="sm" loading={busy === `${b.id}mark_no_show`} onClick={() => void run(b, "mark_no_show")}>
          No show
        </Button>
      )}
    </div>
  );
}

function Row({ block, zone, hour12, now, canWrite, showWho, busy, run, onOpen, open, next }: { block: Block; zone: string; hour12: boolean; now: number; canWrite: boolean; showWho: boolean; busy: string | null; run: Do; onOpen: (id: string) => void; open: string | null; next?: boolean }) {
  const first = block.bookings[0];
  const live = block.start <= now && block.end > now;
  return (
    <li className="bk-row" data-live={live || undefined} data-selected={block.bookings.some((x) => x.id === open) || undefined}>
      <time className="bk-row-time">
        <strong>{clockText(block.start, zone, hour12)}</strong>
        <span className="faint">{clockText(block.end, zone, hour12)}</span>
      </time>
      <div className="bk-row-main">
        <span className="bk-row-line">
          {block.kind === "booking" ? (
            <button type="button" className="bk-row-title" onClick={() => onOpen(first.id)}>
              {first.guest_name ?? "Guest"}
            </button>
          ) : (
            <span className="bk-row-title">{block.title}</span>
          )}
          {next && <Badge tone="blue">Next</Badge>}
        </span>
        <span className="muted">
          {block.kind === "booking" ? first.service_name : `${block.seats} of ${block.capacity} seats`}
          {showWho && first.staff_name ? ` · ${first.staff_name}` : ""}
          {(first.seats ?? 1) > 1 && block.kind === "booking" ? ` · ${first.seats} seats` : ""}
        </span>
        {block.kind === "session" && (
          <ul className="bk-seats">
            {block.bookings.map((x) => (
              <li key={x.id}>
                <button type="button" className="bk-row-title" onClick={() => onOpen(x.id)}>
                  {x.guest_name}
                  {(x.seats ?? 1) > 1 ? ` · ${x.seats}` : ""}
                </button>
                <Badge tone={tone(x.status)}>{x.status}</Badge>
                <Quick b={x} now={now} canWrite={canWrite} busy={busy} run={run} />
              </li>
            ))}
          </ul>
        )}
      </div>
      {block.kind === "booking" && (
        <>
          <Badge tone={tone(first.status)}>{first.status}</Badge>
          <Quick b={first} now={now} canWrite={canWrite} busy={busy} run={run} />
        </>
      )}
    </li>
  );
}

export function Today() {
  useShellTitle("Today");
  const qc = useQueryClient();
  const me = useMe().data;
  const zone = me?.timezone || browserZone();
  const hour12 = useMemo(prefers12Hour, []);
  const [sp, setSp] = useSearchParams();
  const [busy, setBusy] = useState<string | null>(null);
  const now = Date.now();
  const today = dayOf(now, zone);
  const [from, to] = useMemo(() => dayBounds(zone, today), [zone, today]);
  const openId = sp.get("b");

  const meta = useQuery({ queryKey: ["bk", "meta"], queryFn: ({ signal }) => loadMeta(signal), staleTime: 5 * 60_000 });
  const day = useQuery({ queryKey: ["bk", "day", from, to], queryFn: ({ signal }) => loadRange(from, to, signal) });
  const waiting = useQuery({ queryKey: ["bk", "waiting"], queryFn: ({ signal }) => loadWaiting(signal) });
  const bookingMeta = useEntityMeta("Booking");
  const canWrite = !!bookingMeta.data && can(bookingMeta.data.permissions, "write");
  const canCreate = !!bookingMeta.data && can(bookingMeta.data.permissions, "create");
  const refresh = useCallback(() => refreshBookings(qc), [qc]);
  useLiveEntity("Booking", refresh);

  // Their own bookings, when they are someone guests book; the whole day otherwise.
  const myIds = useMemo(() => (meta.data?.staff ?? []).filter((p) => p.email.toLowerCase() === me?.user.email.toLowerCase()).map((p) => p.id), [meta.data, me]);
  const who = sp.get("who") ?? (myIds.length ? "me" : "all");
  const mine = who === "me" && myIds.length > 0;
  const services = useMemo(() => new Map((meta.data?.services ?? []).map((s) => [s.id, s])), [meta.data]);
  const keep = useCallback((b: Bk) => !mine || (!!b.staff && myIds.includes(b.staff)), [mine, myIds]);

  const blocks = useMemo(() => makeBlocks((day.data?.bookings ?? []).filter(keep), services), [day.data, keep, services]);
  const order = useMemo(() => inOrder(blocks.map((b) => ({ ...b, start_time: new Date(b.start).toISOString(), end_time: new Date(b.end).toISOString() })), now), [blocks, now]);
  const asks = (waiting.data ?? []).filter((b) => b.status === "Pending" && keep(b));
  // Over, and still marked confirmed: did they come?
  const open = order.done.filter((b) => b.bookings.some((x) => x.status === "Confirmed" || x.status === "Pending"));
  const closed = order.done.filter((b) => !open.includes(b));

  const run: Do = async (b, action) => {
    setBusy(`${b.id}${action}`);
    try {
      await act(b.id, action);
      toast(action === "confirm" ? `Confirmed ${b.guest_name ?? "the booking"}. They are told.` : action === "complete" ? "Marked as came" : "Marked as a no show");
      refresh();
    } catch (e) {
      toast(errorMessage(e), "error");
    } finally {
      setBusy(null);
    }
  };

  const put = (k: string, v: string | null) =>
    setSp(
      (p) => {
        const n = new URLSearchParams(p);
        if (v === null) n.delete(k);
        else n.set(k, v);
        return n;
      },
      { replace: true },
    );
  const rows = (list: Block[], next?: string) =>
    list.map((b) => <Row key={b.key} block={b} zone={zone} hour12={hour12} now={now} canWrite={canWrite} showWho={!mine} busy={busy} run={run} open={openId} next={b.key === next} onOpen={(id) => put("b", id)} />);

  const loading = !meta.data || !day.data;
  const nothing = !loading && !blocks.length && !asks.length;
  const left = order.now.length + order.next.length;
  return (
    <div className="bk-page bk-today">
      <header className="bk-head">
        <div>
          <Tabs />
          <p className="page-sub" aria-live="polite">
            {dayText(today, { weekday: "long", day: "numeric", month: "long" })}
            {!loading && blocks.length > 0 ? ` · ${left} to go, ${order.done.length} done` : ""}
          </p>
        </div>
        <div className="bk-tools">
          {myIds.length > 0 && (
            <div className="segmented" role="group" aria-label="Whose bookings">
              <button type="button" aria-pressed={mine} onClick={() => put("who", "me")}>
                Mine
              </button>
              <button type="button" aria-pressed={!mine} onClick={() => put("who", "all")}>
                Everyone
              </button>
            </div>
          )}
          {canCreate && (
            <Link className="btn btn-primary btn-md" to="/new">
              <Plus size={16} aria-hidden="true" /> New booking
            </Link>
          )}
        </div>
      </header>

      <div className="bk-layout" data-sheet={openId ? "open" : undefined}>
        <div className="bk-main">
          {meta.error || day.error ? (
            <ErrorState error={meta.error ?? day.error} onRetry={() => { void meta.refetch(); void day.refetch(); }} title="We could not load today" />
          ) : loading ? (
            <PageLoading label="Loading today…" />
          ) : nothing ? (
            <EmptyState title={mine ? "Nothing booked for you today" : "Nothing booked today"} icon="calendar">
              <p className="muted">When guests book, or you book for them, it shows here in the order it happens.</p>
              <Link to="/">Open the schedule</Link>
            </EmptyState>
          ) : (
            <div className="bk-lists">
              {asks.length > 0 && (
                <section aria-label="Needs your answer">
                  <h2 className="bk-group">
                    Needs your answer <Badge tone="amber">{asks.length}</Badge>
                  </h2>
                  <ul className="bk-rows">
                    {asks.map((b) => (
                      <li key={b.id} className="bk-row">
                        <time className="bk-row-time">
                          <strong>{clockText(Date.parse(b.start_time), zone, hour12)}</strong>
                          <span className="faint">{dayText(dayOf(Date.parse(b.start_time), zone), { weekday: "short", day: "numeric", month: "short" })}</span>
                        </time>
                        <div className="bk-row-main">
                          <button type="button" className="bk-row-title" onClick={() => put("b", b.id)}>
                            {b.guest_name ?? "Guest"}
                          </button>
                          <span className="muted">
                            {b.service_name}
                            {b.staff_name ? ` · ${b.staff_name}` : ""}
                          </span>
                        </div>
                        <Quick b={b} now={now} canWrite={canWrite} busy={busy} run={run} />
                      </li>
                    ))}
                  </ul>
                </section>
              )}
              {open.length > 0 && (
                <section aria-label="Needs closing">
                  <h2 className="bk-group">Did they come? <Badge tone="amber">{open.length}</Badge></h2>
                  <ul className="bk-rows">{rows(open)}</ul>
                </section>
              )}
              {order.now.length > 0 && (
                <section aria-label="Under way">
                  <h2 className="bk-group">Under way</h2>
                  <ul className="bk-rows">{rows(order.now)}</ul>
                </section>
              )}
              {order.next.length > 0 && (
                <section aria-label="Coming up">
                  <h2 className="bk-group">Coming up</h2>
                  <ul className="bk-rows">{rows(order.next, order.next[0]?.key)}</ul>
                </section>
              )}
              {closed.length > 0 && (
                <section aria-label="Done">
                  <h2 className="bk-group">Done</h2>
                  <ul className="bk-rows">{rows(closed)}</ul>
                </section>
              )}
            </div>
          )}
        </div>
        {openId && (
          <BookingSheet
            key={openId}
            id={openId}
            zone={zone}
            hour12={hour12}
            canWrite={canWrite}
            onClose={() => put("b", null)}
          />
        )}
      </div>
    </div>
  );
}

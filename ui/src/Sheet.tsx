// A booking, beside the schedule or the day's list: when, who, where, what they answered, what it
// costs, and what can be done to it now (the buttons follow the server's rules for its status).
// Moving it shows real free times; cancelling asks for a reason and says whether the guest is told.
// A class session lists its seats. Every change comes back from the server as the booking as it now
// stands.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Mail, MapPin, Phone, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { api, docPath, errorMessage, formatMoment } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, Dialog, ErrorState, Field, Menu, MenuItem, Spinner, TabPanel, Tabs, toast } from "@ferrum/kit";
import { type Bk, nextSteps, type Step, tone } from "./board";
import { act } from "./data";
import { priceText } from "./guest/booking";
import { Reschedule } from "./Reschedule";
import { clockText, dayOf, dayText } from "./zone";

type Full = Bk & { answers?: { question: string; answer: string }[]; guest_time_zone?: string | null };

/** Everything that shows a booking asks again after a change. */
export const refreshBookings = (qc: ReturnType<typeof useQueryClient>) => {
  for (const k of ["range", "waiting", "doc", "day", "audit"]) void qc.invalidateQueries({ queryKey: ["bk", k] });
};

const span = (ms: number) => {
  const m = Math.max(1, Math.round(ms / 60_000));
  if (m < 90) return `${m} minute${m === 1 ? "" : "s"}`;
  if (m < 36 * 60) return `${Math.round(m / 60)} hours`;
  return `${Math.round(m / 1440)} days`;
};

const HISTORY: Record<string, string> = {
  create: "Booked",
  "action:confirm": "Confirmed",
  "action:reschedule": "Moved to another time",
  "action:cancel_booking": "Cancelled",
  "action:complete": "Marked completed",
  "action:mark_no_show": "Marked as no show",
  "action:make_sales_invoice": "Invoice created",
  "action:guest_link": "Guest link made",
  "action:repeat": "Repeated",
  "action:cancel_series": "Series cancelled",
};

function History({ id }: { id: string }) {
  const q = useQuery({
    queryKey: ["bk", "audit", id],
    queryFn: ({ signal }) => api.get<{ events: { at: string; actor_name?: string | null; actor?: string | null; action: string }[] }>(docPath("Booking", id, "audit"), undefined, signal),
  });
  if (q.isLoading) return <Spinner label="Loading the history" />;
  if (q.error) return <p className="muted">The history is not available to you.</p>;
  const events = (q.data?.events ?? []).filter((e) => HISTORY[e.action]).reverse();
  if (!events.length) return <p className="muted">Nothing has happened to it yet.</p>;
  return (
    <ol className="bk-history">
      {events.map((e, i) => (
        <li key={`${e.at}-${i}`}>
          <strong>{HISTORY[e.action]}</strong>
          <span className="faint">
            {formatMoment(e.at, { day: "numeric", month: "short", hour: "numeric", minute: "2-digit" })}
            {e.actor_name || e.actor ? ` · ${e.actor_name ?? e.actor}` : ""}
          </span>
        </li>
      ))}
    </ol>
  );
}

function Cancel({ b, onClose, onDone }: { b: Full; onClose: () => void; onDone: (b: Bk) => void }) {
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const told = b.guest_email ? `${b.guest_name ?? "The guest"} is told by email at ${b.guest_email}.` : "There is no email address for the guest, so they are not told.";
  const go = async () => {
    setBusy(true);
    setError(null);
    try {
      onDone(await act(b.id, "cancel_booking", { reason: reason.trim() || undefined }));
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };
  return (
    <div className="stack">
      <h3>{b.status === "Pending" ? "Decline this booking?" : "Cancel this booking?"}</h3>
      <p className="muted">The time is freed. {told}</p>
      <Field label="Reason (optional)" htmlFor="bk-reason" hint="The guest sees it.">
        <textarea id="bk-reason" className="textarea" rows={3} maxLength={1000} value={reason} onChange={(e) => setReason(e.target.value)} />
      </Field>
      {error && <Alert tone="error">{error}</Alert>}
      <div className="bk-sheet-actions">
        <Button onClick={onClose}>Keep it</Button>
        <Button variant="danger" loading={busy} onClick={() => void go()}>
          {b.status === "Pending" ? "Decline booking" : "Cancel booking"}
        </Button>
      </div>
    </div>
  );
}

function Repeat({ b, onClose, onDone }: { b: Full; onClose: () => void; onDone: () => void }) {
  const [every, setEvery] = useState("week");
  const [count, setCount] = useState(4);
  const [skip, setSkip] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const go = async () => {
    setBusy(true);
    setError(null);
    try {
      const out = await act<{ booked: unknown[]; skipped: { date: string; reason: string }[] }>(b.id, "repeat", { every, count, skip_conflicts: skip });
      toast(out.skipped.length ? `Repeated: ${out.booked.length} booked, ${out.skipped.length} skipped (${out.skipped.map((s) => s.date).join(", ")})` : `Repeated: ${out.booked.length} more booked`);
      onDone();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };
  return (
    <Dialog
      open
      onClose={onClose}
      title="Repeat this booking"
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" loading={busy} onClick={() => void go()}>
            Repeat
          </Button>
        </>
      }
    >
      <div className="stack">
        {error && <Alert tone="error">{error}</Alert>}
        <p className="muted">Each date is booked at the same time of day, checked like any booking. They are made together or not at all, unless you skip the dates that do not fit.</p>
        <Field label="How often" htmlFor="bk-every">
          <select id="bk-every" className="select" value={every} onChange={(e) => setEvery(e.target.value)}>
            <option value="week">Every week</option>
            <option value="2 weeks">Every 2 weeks</option>
            <option value="4 weeks">Every 4 weeks</option>
            <option value="month">Every month</option>
          </select>
        </Field>
        <Field label="How many bookings in all" htmlFor="bk-count" hint="This one included, up to 52.">
          <input id="bk-count" className="input" type="number" min={2} max={52} value={count} onChange={(e) => setCount(Math.min(52, Math.max(2, Number(e.target.value) || 2)))} />
        </Field>
        <label className="check-field">
          <input type="checkbox" checked={skip} onChange={(e) => setSkip(e.target.checked)} /> Skip the dates that do not fit
        </label>
      </div>
    </Dialog>
  );
}

function GuestLink({ id, onClose }: { id: string; onClose: () => void }) {
  const q = useQuery({ queryKey: ["bk", "link", id], queryFn: () => act<{ url: string; note: string; expires_at: string }>(id, "guest_link"), staleTime: Infinity, gcTime: 0 });
  return (
    <Dialog open onClose={onClose} title="Link for the guest" footer={<Button onClick={onClose}>Done</Button>}>
      {q.isLoading ? (
        <Spinner label="Making the link" />
      ) : q.error ? (
        <Alert tone="error">{errorMessage(q.error)}</Alert>
      ) : (
        <div className="stack">
          <div className="secret">
            <input className="input" readOnly value={q.data!.url} aria-label="The guest's link" onFocus={(e) => e.currentTarget.select()} />
            <Button
              icon="copy"
              onClick={() =>
                void navigator.clipboard
                  .writeText(q.data!.url)
                  .then(() => toast("Link copied"))
                  .catch(() => toast("Select the link and copy it", "error"))
              }
            >
              Copy
            </Button>
          </div>
          <p className="muted">{q.data!.note}</p>
        </div>
      )}
    </Dialog>
  );
}

export function BookingSheet({ id, zone, hour12, canWrite, onClose, onChanged }: { id: string; zone: string; hour12: boolean; canWrite: boolean; onClose: () => void; onChanged?: (b: Bk) => void }) {
  const qc = useQueryClient();
  const head = useRef<HTMLHeadingElement>(null);
  const [tab, setTab] = useState("info");
  const [mode, setMode] = useState<"info" | "move" | "cancel">("info");
  const [dialog, setDialog] = useState<"link" | "repeat" | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const q = useQuery({ queryKey: ["bk", "doc", id], queryFn: ({ signal }) => api.get<Full>(docPath("Booking", id), { titles: true }, signal) });
  const b = q.data;
  const now = Date.now();

  // A booking opened from the schedule takes the focus; closing it gives the focus back.
  useEffect(() => {
    head.current?.focus();
    setTab("info");
    setMode("info");
  }, [id]);

  const changed = (next: Bk) => {
    qc.setQueryData<Full>(["bk", "doc", id], (old) => (old ? { ...old, ...next } : (next as Full)));
    refreshBookings(qc);
    onChanged?.(next);
    setMode("info");
  };

  const run = async (name: string, params: Record<string, unknown> = {}, done?: string) => {
    setBusy(name);
    try {
      changed(await act(id, name, params));
      if (done) toast(done);
    } catch (e) {
      toast(errorMessage(e), "error");
    } finally {
      setBusy(null);
    }
  };

  const noShow = async () => {
    const ok = await confirmDialog({ title: `Mark ${b?.guest_name ?? "the guest"} as a no show?`, message: "It counts against them in the attendance report. The guest is not told.", confirmLabel: "Mark as no show", danger: true });
    if (ok) void run("mark_no_show", {}, "Marked as a no show");
  };

  const invoice = async () => {
    setBusy("make_sales_invoice");
    try {
      await act(id, "make_sales_invoice");
      toast("A draft sales invoice was made");
      void qc.invalidateQueries({ queryKey: ["bk", "doc", id] });
    } catch (e) {
      toast(errorMessage(e), "error");
    } finally {
      setBusy(null);
    }
  };

  const steps: Step[] = b && canWrite ? nextSteps(b, now) : [];
  const start = b ? Date.parse(b.start_time) : 0;
  const end = b ? Date.parse(b.end_time) : 0;
  const note = !b ? "" : now < start ? `Starts in ${span(start - now)}` : now < end ? "Under way" : `Ended ${span(now - end)} ago`;
  const primary: Step | undefined = steps.includes("confirm") ? "confirm" : steps.includes("complete") ? "complete" : steps.includes("reschedule") ? "reschedule" : undefined;

  return (
    <aside className="bk-sheet" aria-label="Booking" onKeyDown={(e) => e.key === "Escape" && !e.defaultPrevented && !dialog && onClose()}>
      <header className="bk-sheet-head">
        <div>
          <h2 ref={head} tabIndex={-1}>
            {b ? (b.guest_name ?? "Booking") : "Booking"}
          </h2>
          {b && (
            <p className="muted">
              {b.service_name} <Badge tone={tone(b.status)}>{b.status}</Badge>
            </p>
          )}
        </div>
        <button type="button" className="icon-btn" aria-label="Close" title="Close (Esc)" onClick={onClose}>
          <X size={18} />
        </button>
      </header>
      {q.error ? (
        <ErrorState error={q.error} onRetry={() => void q.refetch()} title="We could not open this booking" />
      ) : !b ? (
        <div className="bk-center">
          <Spinner label="Loading the booking" />
        </div>
      ) : (
        <>
          <Tabs
            id="bk-sheet"
            tabs={[
              { id: "info", label: "Details" },
              { id: "history", label: "History" },
            ]}
            value={tab}
            onChange={setTab}
          />
          <TabPanel id="bk-sheet" tab={tab}>
            <div className="bk-sheet-body">
              {tab === "history" ? (
                <History id={id} />
              ) : mode === "move" ? (
                <Reschedule booking={b} zone={zone} hour12={hour12} onClose={() => setMode("info")} onDone={(next) => { changed(next); toast("Moved"); }} />
              ) : mode === "cancel" ? (
                <Cancel b={b} onClose={() => setMode("info")} onDone={(next) => { changed(next); toast(b.status === "Pending" ? "Declined" : "Cancelled"); }} />
              ) : (
                <>
                  <section className="bk-part">
                    <h3>When</h3>
                    <p className="bk-big">{dayText(dayOf(start, zone), { weekday: "long", day: "numeric", month: "long", year: "numeric" })}</p>
                    <p>
                      {clockText(start, zone, hour12)} – {clockText(end, zone, hour12)} <span className="faint">({Math.round((end - start) / 60_000)} min)</span>
                    </p>
                    {(b.status === "Pending" || b.status === "Confirmed") && <p className="faint">{note}</p>}
                    {b.series && <p className="faint">Part of a repeating series.</p>}
                  </section>
                  <section className="bk-part">
                    <h3>Who</h3>
                    <p className="bk-big">{b.guest_name ?? "—"}</p>
                    {b.guest_email && (
                      <p>
                        <Mail size={13} aria-hidden="true" /> <a href={`mailto:${b.guest_email}`}>{b.guest_email}</a>
                      </p>
                    )}
                    {b.guest_phone && (
                      <p>
                        <Phone size={13} aria-hidden="true" /> <a href={`tel:${b.guest_phone}`}>{b.guest_phone}</a>
                      </p>
                    )}
                    <p className="faint">
                      With {b.staff_name ?? "—"}
                      {(b.seats ?? 1) > 1 ? ` · ${b.seats} seats` : ""}
                      {b.source === "Public" ? " · booked online" : ""}
                    </p>
                    {b.customer && (
                      <p>
                        <a href={`/open/Customer/${b.customer}`}>{b._titles?.customer ?? "Customer"}</a>
                      </p>
                    )}
                  </section>
                  {(b.location_type || b.location || b.resource) && (
                    <section className="bk-part">
                      <h3>Where</h3>
                      <p>
                        <MapPin size={13} aria-hidden="true" /> {b.location_type}
                        {b.location ? ` · ${b.location}` : ""}
                      </p>
                      {b.resource && <p className="faint">{b._titles?.resource ?? "A room"}</p>}
                    </section>
                  )}
                  {(b.answers?.length || b.notes) && (
                    <section className="bk-part">
                      <h3>What they said</h3>
                      <dl className="bk-answers">
                        {b.answers?.map((a) => (
                          <div key={a.question}>
                            <dt>{a.question}</dt>
                            <dd>{a.answer}</dd>
                          </div>
                        ))}
                        {b.notes && (
                          <div>
                            <dt>Notes</dt>
                            <dd>{b.notes}</dd>
                          </div>
                        )}
                      </dl>
                    </section>
                  )}
                  {b.status === "Cancelled" && b.cancellation_reason && (
                    <section className="bk-part">
                      <h3>Why it was cancelled</h3>
                      <p>{b.cancellation_reason}</p>
                    </section>
                  )}
                  <section className="bk-part">
                    <h3>Billing</h3>
                    <p>
                      {Number(b.amount) > 0 ? priceText(b.amount, null) : "Free"}
                      {b.sales_invoice ? (
                        <>
                          {" · "}
                          <a href={`/open/SalesInvoice/${b.sales_invoice}`}>{b._titles?.sales_invoice ?? "Invoice"}</a>
                        </>
                      ) : null}
                    </p>
                    {canWrite && !b.sales_invoice && Number(b.amount) > 0 && ["Confirmed", "Completed", "No Show"].includes(b.status) && (
                      <Button size="sm" loading={busy === "make_sales_invoice"} onClick={() => void invoice()}>
                        Create invoice
                      </Button>
                    )}
                  </section>
                  <p className="faint bk-number">
                    {b.number} · <Link to={`/e/Booking/${b.id}`}>Open the full record</Link>
                  </p>
                </>
              )}
            </div>
          </TabPanel>
          {mode === "info" && tab === "info" && canWrite && steps.length > 0 && (
            <footer className="bk-sheet-foot">
              {steps.includes("confirm") && (
                <Button variant={primary === "confirm" ? "primary" : "default"} loading={busy === "confirm"} onClick={() => void run("confirm", {}, "Confirmed. The guest is told.")}>
                  Confirm
                </Button>
              )}
              {steps.includes("complete") && (
                <Button variant={primary === "complete" ? "primary" : "default"} loading={busy === "complete"} onClick={() => void run("complete", {}, "Marked completed")}>
                  Completed
                </Button>
              )}
              {steps.includes("mark_no_show") && (
                <Button loading={busy === "mark_no_show"} onClick={() => void noShow()}>
                  No show
                </Button>
              )}
              {steps.includes("reschedule") && (
                <Button variant={primary === "reschedule" ? "primary" : "default"} onClick={() => setMode("move")}>
                  Move
                </Button>
              )}
              {steps.includes("cancel_booking") && (
                <Button variant="danger" onClick={() => setMode("cancel")}>
                  {b.status === "Pending" ? "Decline" : "Cancel"}
                </Button>
              )}
              <span className="spacer" />
              <Menu ariaLabel="More about this booking" label="More" buttonClass="btn btn-default btn-md">
                {(close) => (
                  <>
                    {b.status !== "Cancelled" && (
                      <MenuItem onSelect={() => { close(); setDialog("link"); }}>Link for the guest…</MenuItem>
                    )}
                    {!b.series && (b.status === "Pending" || b.status === "Confirmed") && (
                      <MenuItem onSelect={() => { close(); setDialog("repeat"); }}>Repeat…</MenuItem>
                    )}
                    {b.series && b.status !== "Cancelled" && (
                      <MenuItem
                        danger
                        onSelect={async () => {
                          close();
                          if (await confirmDialog({ title: "Cancel this and the later bookings?", message: "Every booking of this series from this one on is cancelled, and each guest is told.", confirmLabel: "Cancel them", danger: true })) void run("cancel_series", {}, "The series is cancelled");
                        }}
                      >
                        Cancel this and later…
                      </MenuItem>
                    )}
                  </>
                )}
              </Menu>
            </footer>
          )}
        </>
      )}
      {dialog === "link" && <GuestLink id={id} onClose={() => setDialog(null)} />}
      {dialog === "repeat" && b && (
        <Repeat
          b={b}
          onClose={() => setDialog(null)}
          onDone={() => {
            setDialog(null);
            refreshBookings(qc);
          }}
        />
      )}
    </aside>
  );
}

/** A class session: who has a seat, and a way into each booking. */
export function SessionSheet({ bookings, zone, hour12, onOpen, onClose }: { bookings: Bk[]; zone: string; hour12: boolean; onOpen: (id: string) => void; onClose: () => void }) {
  const head = useRef<HTMLHeadingElement>(null);
  useEffect(() => head.current?.focus(), []);
  const first = bookings[0];
  const start = Date.parse(first.start_time);
  const end = Date.parse(first.end_time);
  const seats = bookings.reduce((n, x) => n + (x.seats ?? 1), 0);
  return (
    <aside className="bk-sheet" aria-label="Class" onKeyDown={(e) => e.key === "Escape" && onClose()}>
      <header className="bk-sheet-head">
        <div>
          <h2 ref={head} tabIndex={-1}>
            {first.service_name}
          </h2>
          <p className="muted">
            {dayText(dayOf(start, zone), { weekday: "long", day: "numeric", month: "long" })}, {clockText(start, zone, hour12)} – {clockText(end, zone, hour12)} · with {first.staff_name}
          </p>
        </div>
        <button type="button" className="icon-btn" aria-label="Close" title="Close (Esc)" onClick={onClose}>
          <X size={18} />
        </button>
      </header>
      <div className="bk-sheet-body">
        <p className="bk-big">
          {seats} {seats === 1 ? "seat" : "seats"} taken
        </p>
        <ul className="bk-attendees">
          {bookings.map((x) => (
            <li key={x.id}>
              <button type="button" className="bk-attendee" onClick={() => onOpen(x.id)}>
                <span>
                  <strong>{x.guest_name}</strong>
                  {(x.seats ?? 1) > 1 && <span className="faint"> · {x.seats} seats</span>}
                </span>
                <Badge tone={tone(x.status)}>{x.status}</Badge>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </aside>
  );
}

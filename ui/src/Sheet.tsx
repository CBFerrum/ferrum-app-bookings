// A booking, beside the schedule or the day's list: when, who, where, what they answered, what it
// costs, and what can be done to it now (the buttons follow the server's rules for its status).
// Moving it shows real free times; cancelling asks for a reason and says whether the guest is told.
// A class session lists its seats. Every change comes back from the server as the booking as it now
// stands.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Mail, MapPin, Phone, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { api, docPath, errorMessage, formatMoment, t, isolate } from "@ferrum/client";
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
  if (m < 90) return (m === 1 ? t("1 minute") : t("{m} minutes", { m }));
  if (m < 36 * 60) return t("{n} hours", { n: Math.round(m / 60) });
  return t("{n} days", { n: Math.round(m / 1440) });
};

const HISTORY: Record<string, string> = {
  get create() { return t("Booked"); },
  get "action:confirm"() { return t("Confirmed"); },
  get "action:reschedule"() { return t("Moved to another time"); },
  get "action:cancel_booking"() { return t("Cancelled"); },
  get "action:complete"() { return t("Marked completed"); },
  get "action:mark_no_show"() { return t("Marked as no show"); },
  get "action:make_sales_invoice"() { return t("Invoice created"); },
  get "action:guest_link"() { return t("Guest link made"); },
  get "action:repeat"() { return t("Repeated"); },
  get "action:cancel_series"() { return t("Series cancelled"); },
};

function History({ id }: { id: string }) {
  const q = useQuery({
    queryKey: ["bk", "audit", id],
    queryFn: ({ signal }) => api.get<{ events: { at: string; actor_name?: string | null; actor?: string | null; action: string }[] }>(docPath("Booking", id, "audit"), undefined, signal),
  });
  if (q.isLoading) return <Spinner label={t("Loading the history")} />;
  if (q.error) return <p className="muted">{t("The history is not available to you.")}</p>;
  const events = (q.data?.events ?? []).filter((e) => HISTORY[e.action]).reverse();
  if (!events.length) return <p className="muted">{t("Nothing has happened to it yet.")}</p>;
  return (
    <ol className="bk-history">
      {events.map((e, i) => (
        <li key={`${e.at}-${i}`}>
          <strong>{isolate(HISTORY[e.action])}</strong>
          <span className="faint">
            {isolate(formatMoment(e.at, { day: "numeric", month: "short", hour: "numeric", minute: "2-digit" }))}
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
  const told = b.guest_email ? t("{value} is told by email at {guest_email}.", { value: b.guest_name ?? t("The guest"), guest_email: b.guest_email }) : t("There is no email address for the guest, so they are not told.");
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
      <h3>{b.status === "Pending" ? t("Decline this booking?") : t("Cancel this booking?")}</h3>
      <p className="muted">{t("The time is freed. {told}", { told })}</p>
      <Field label={t("Reason (optional)")} htmlFor="bk-reason" hint={t("The guest sees it.")}>
        <textarea id="bk-reason" className="textarea" rows={3} maxLength={1000} value={reason} onChange={(e) => setReason(e.target.value)} />
      </Field>
      {error && <Alert tone="error">{isolate(error)}</Alert>}
      <div className="bk-sheet-actions">
        <Button onClick={onClose}>{t("Keep it")}</Button>
        <Button variant="danger" loading={busy} onClick={() => void go()}>
          {b.status === "Pending" ? t("Decline booking") : t("Cancel booking")}
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
      toast(out.skipped.length ? t("Repeated: {length} booked, {length2} skipped ({value})", { length: out.booked.length, length2: out.skipped.length, value: out.skipped.map((s) => s.date).join(", ") }) : t("Repeated: {length} more booked", { length: out.booked.length }));
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
      title={t("Repeat this booking")}
      footer={
        <>
          <Button onClick={onClose}>{t("Cancel")}</Button>
          <Button variant="primary" loading={busy} onClick={() => void go()}>
            {t("Repeat")}
          </Button>
        </>
      }
    >
      <div className="stack">
        {error && <Alert tone="error">{isolate(error)}</Alert>}
        <p className="muted">{t("Each date is booked at the same time of day, checked like any booking. They are made together or not at all, unless you skip the dates that do not fit.")}</p>
        <Field label={t("How often")} htmlFor="bk-every">
          <select id="bk-every" className="select" value={every} onChange={(e) => setEvery(e.target.value)}>
            <option value="week">{t("Every week")}</option>
            <option value="2 weeks">{t("Every 2 weeks")}</option>
            <option value="4 weeks">{t("Every 4 weeks")}</option>
            <option value="month">{t("Every month")}</option>
          </select>
        </Field>
        <Field label={t("How many bookings in all")} htmlFor="bk-count" hint={t("This one included, up to 52.")}>
          <input id="bk-count" className="input" type="number" min={2} max={52} value={count} onChange={(e) => setCount(Math.min(52, Math.max(2, Number(e.target.value) || 2)))} />
        </Field>
        <label className="check-field">
          <input type="checkbox" checked={skip} onChange={(e) => setSkip(e.target.checked)} /> {t("Skip the dates that do not fit")}
        </label>
      </div>
    </Dialog>
  );
}

function GuestLink({ id, onClose }: { id: string; onClose: () => void }) {
  const q = useQuery({ queryKey: ["bk", "link", id], queryFn: () => act<{ url: string; note: string; expires_at: string }>(id, "guest_link"), staleTime: Infinity, gcTime: 0 });
  return (
    <Dialog open onClose={onClose} title={t("Link for the guest")} footer={<Button onClick={onClose}>{t("Done")}</Button>}>
      {q.isLoading ? (
        <Spinner label={t("Making the link")} />
      ) : q.error ? (
        <Alert tone="error">{isolate(errorMessage(q.error))}</Alert>
      ) : (
        <div className="stack">
          <div className="secret">
            <input className="input" readOnly value={q.data!.url} aria-label={t("The guest's link")} onFocus={(e) => e.currentTarget.select()} />
            <Button
              icon="copy"
              onClick={() =>
                void navigator.clipboard
                  .writeText(q.data!.url)
                  .then(() => toast(t("Link copied")))
                  .catch(() => toast(t("Select the link and copy it"), "error"))
              }
            >
              {t("Copy")}
            </Button>
          </div>
          <p className="muted">{isolate(q.data!.note)}</p>
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
    const ok = await confirmDialog({ title: t("Mark {value} as a no show?", { value: b?.guest_name ?? t("the guest") }), message: t("It counts against them in the attendance report. The guest is not told."), confirmLabel: t("Mark as no show"), danger: true });
    if (ok) void run("mark_no_show", {}, t("Marked as a no show"));
  };

  const invoice = async () => {
    setBusy("make_sales_invoice");
    try {
      await act(id, "make_sales_invoice");
      toast(t("A draft sales invoice was made"));
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
  const note = !b ? "" : now < start ? t("Starts in {value}", { value: span(start - now) }) : now < end ? t("Under way") : t("Ended {value} ago", { value: span(now - end) });
  const primary: Step | undefined = steps.includes("confirm") ? "confirm" : steps.includes("complete") ? "complete" : steps.includes("reschedule") ? "reschedule" : undefined;

  return (
    <aside className="bk-sheet" aria-label={t("Booking")} onKeyDown={(e) => e.key === "Escape" && !e.defaultPrevented && !dialog && onClose()}>
      <header className="bk-sheet-head">
        <div>
          <h2 ref={head} tabIndex={-1}>
            {b ? (b.guest_name ?? t("Booking")) : t("Booking")}
          </h2>
          {b && (
            <p className="muted">
              {isolate(b.service_name ?? "")} <Badge tone={tone(b.status)}>{isolate(b.status)}</Badge>
            </p>
          )}
        </div>
        <button type="button" className="icon-btn" aria-label={t("Close")} title={t("Close (Esc)")} onClick={onClose}>
          <X size={18} />
        </button>
      </header>
      {q.error ? (
        <ErrorState error={q.error} onRetry={() => void q.refetch()} title={t("We could not open this booking")} />
      ) : !b ? (
        <div className="bk-center">
          <Spinner label={t("Loading the booking")} />
        </div>
      ) : (
        <>
          <Tabs
            id="bk-sheet"
            tabs={[
              { id: "info", label: t("Details") },
              { id: "history", label: t("History") },
            ]}
            value={tab}
            onChange={setTab}
          />
          <TabPanel id="bk-sheet" tab={tab}>
            <div className="bk-sheet-body">
              {tab === "history" ? (
                <History id={id} />
              ) : mode === "move" ? (
                <Reschedule booking={b} zone={zone} hour12={hour12} onClose={() => setMode("info")} onDone={(next) => { changed(next); toast(t("Moved")); }} />
              ) : mode === "cancel" ? (
                <Cancel b={b} onClose={() => setMode("info")} onDone={(next) => { changed(next); toast(b.status === "Pending" ? t("Declined") : t("Cancelled")); }} />
              ) : (
                <>
                  <section className="bk-part">
                    <h3>{t("When")}</h3>
                    <p className="bk-big">{isolate(dayText(dayOf(start, zone), { weekday: "long", day: "numeric", month: "long", year: "numeric" }))}</p>
                    <p>
                      {isolate(clockText(start, zone, hour12))} – {isolate(clockText(end, zone, hour12))} <span className="faint">({Math.round((end - start) / 60_000)} {t("min)")}</span>
                    </p>
                    {(b.status === "Pending" || b.status === "Confirmed") && <p className="faint">{isolate(note)}</p>}
                    {b.series && <p className="faint">{t("Part of a repeating series.")}</p>}
                  </section>
                  <section className="bk-part">
                    <h3>{t("Who")}</h3>
                    <p className="bk-big">{b.guest_name ?? "—"}</p>
                    {b.guest_email && (
                      <p>
                        <Mail size={13} aria-hidden="true" /> <a href={`mailto:${b.guest_email}`}>{isolate(b.guest_email)}</a>
                      </p>
                    )}
                    {b.guest_phone && (
                      <p>
                        <Phone size={13} aria-hidden="true" /> <a href={`tel:${b.guest_phone}`}>{isolate(b.guest_phone)}</a>
                      </p>
                    )}
                    <p className="faint">
                      {t("With")} {b.staff_name ?? "—"}
                      {(b.seats ?? 1) > 1 ? t(" · {seats} seats", { seats: String(b.seats) }) : ""}
                      {b.source === "Public" ? t(" · booked online") : ""}
                    </p>
                    {b.customer && (
                      <p>
                        <a href={`/open/Customer/${b.customer}`}>{b._titles?.customer ?? t("Customer")}</a>
                      </p>
                    )}
                  </section>
                  {(b.location_type || b.location || b.resource) && (
                    <section className="bk-part">
                      <h3>{t("Where")}</h3>
                      <p>
                        <MapPin size={13} aria-hidden="true" /> {isolate(b.location_type ?? "")}
                        {b.location ? ` · ${b.location}` : ""}
                      </p>
                      {b.resource && <p className="faint">{b._titles?.resource ?? t("A room")}</p>}
                    </section>
                  )}
                  {(b.answers?.length || b.notes) && (
                    <section className="bk-part">
                      <h3>{t("What they said")}</h3>
                      <dl className="bk-answers">
                        {b.answers?.map((a) => (
                          <div key={a.question}>
                            <dt>{isolate(a.question)}</dt>
                            <dd>{isolate(a.answer)}</dd>
                          </div>
                        ))}
                        {b.notes && (
                          <div>
                            <dt>{t("Notes")}</dt>
                            <dd>{isolate(b.notes)}</dd>
                          </div>
                        )}
                      </dl>
                    </section>
                  )}
                  {b.status === "Cancelled" && b.cancellation_reason && (
                    <section className="bk-part">
                      <h3>{t("Why it was cancelled")}</h3>
                      <p>{isolate(b.cancellation_reason)}</p>
                    </section>
                  )}
                  <section className="bk-part">
                    <h3>{t("Billing")}</h3>
                    <p>
                      {Number(b.amount) > 0 ? priceText(b.amount, null) : t("Free")}
                      {b.sales_invoice ? (
                        <>
                          {isolate(" · ")}
                          <a href={`/open/SalesInvoice/${b.sales_invoice}`}>{b._titles?.sales_invoice ?? t("Invoice")}</a>
                        </>
                      ) : null}
                    </p>
                    {canWrite && !b.sales_invoice && Number(b.amount) > 0 && ["Confirmed", "Completed", "No Show"].includes(b.status) && (
                      <Button size="sm" loading={busy === "make_sales_invoice"} onClick={() => void invoice()}>
                        {t("Create invoice")}
                      </Button>
                    )}
                  </section>
                  <p className="faint bk-number">
                    {isolate(b.number ?? "")} · <Link to={`/e/Booking/${b.id}`}>{t("Open the full record")}</Link>
                  </p>
                </>
              )}
            </div>
          </TabPanel>
          {mode === "info" && tab === "info" && canWrite && steps.length > 0 && (
            <footer className="bk-sheet-foot">
              {steps.includes("confirm") && (
                <Button variant={primary === "confirm" ? "primary" : "default"} loading={busy === "confirm"} onClick={() => void run("confirm", {}, t("Confirmed. The guest is told."))}>
                  {t("Confirm")}
                </Button>
              )}
              {steps.includes("complete") && (
                <Button variant={primary === "complete" ? "primary" : "default"} loading={busy === "complete"} onClick={() => void run("complete", {}, t("Marked completed"))}>
                  {t("Completed")}
                </Button>
              )}
              {steps.includes("mark_no_show") && (
                <Button loading={busy === "mark_no_show"} onClick={() => void noShow()}>
                  {t("No show")}
                </Button>
              )}
              {steps.includes("reschedule") && (
                <Button variant={primary === "reschedule" ? "primary" : "default"} onClick={() => setMode("move")}>
                  {t("Move")}
                </Button>
              )}
              {steps.includes("cancel_booking") && (
                <Button variant="danger" onClick={() => setMode("cancel")}>
                  {b.status === "Pending" ? t("Decline") : t("Cancel")}
                </Button>
              )}
              <span className="spacer" />
              <Menu ariaLabel={t("More about this booking")} label={t("More")} buttonClass="btn btn-default btn-md">
                {(close) => (
                  <>
                    {b.status !== "Cancelled" && (
                      <MenuItem onSelect={() => { close(); setDialog("link"); }}>{t("Link for the guest…")}</MenuItem>
                    )}
                    {!b.series && (b.status === "Pending" || b.status === "Confirmed") && (
                      <MenuItem onSelect={() => { close(); setDialog("repeat"); }}>{t("Repeat…")}</MenuItem>
                    )}
                    {b.series && b.status !== "Cancelled" && (
                      <MenuItem
                        danger
                        onSelect={async () => {
                          close();
                          if (await confirmDialog({ title: t("Cancel this and the later bookings?"), message: t("Every booking of this series from this one on is cancelled, and each guest is told."), confirmLabel: t("Cancel them"), danger: true })) void run("cancel_series", {}, t("The series is cancelled"));
                        }}
                      >
                        {t("Cancel this and later…")}
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
    <aside className="bk-sheet" aria-label={t("Class")} onKeyDown={(e) => e.key === "Escape" && onClose()}>
      <header className="bk-sheet-head">
        <div>
          <h2 ref={head} tabIndex={-1}>
            {isolate(first.service_name ?? "")}
          </h2>
          <p className="muted">
            {isolate(dayText(dayOf(start, zone), { weekday: "long", day: "numeric", month: "long" }))}, {isolate(clockText(start, zone, hour12))} – {isolate(clockText(end, zone, hour12))} {t("· with {staff_name}", { staff_name: first.staff_name ?? "" })}
          </p>
        </div>
        <button type="button" className="icon-btn" aria-label={t("Close")} title={t("Close (Esc)")} onClick={onClose}>
          <X size={18} />
        </button>
      </header>
      <div className="bk-sheet-body">
        <p className="bk-big">
          {seats} {(seats === 1 ? t("seat taken") : t("seats taken"))}
        </p>
        <ul className="bk-attendees">
          {bookings.map((x) => (
            <li key={x.id}>
              <button type="button" className="bk-attendee" onClick={() => onOpen(x.id)}>
                <span>
                  <strong>{isolate(x.guest_name ?? "")}</strong>
                  {(x.seats ?? 1) > 1 && <span className="faint"> {t("· {seats} seats", { seats: x.seats ?? "" })}</span>}
                </span>
                <Badge tone={tone(x.status)}>{isolate(x.status)}</Badge>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </aside>
  );
}

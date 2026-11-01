// A guest's own booking (/bookings/book/{workspace}/manage/{token}, the link in their email): what
// was booked and where, a way to put it in a calendar, and, while the business still allows it, a
// move to another time or a cancellation. The page shows what the server says (its windows, its
// refusals); the link is the guest's only key, so it names nothing but their booking.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { CalendarPlus, CircleCheck, Hourglass, CalendarX, Mail } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { Link, useLocation, useParams } from "react-router";
import { errorMessage } from "@ferrum/client";
import { Alert, Badge, Button, Dialog, Field, toast, useShellTitle } from "@ferrum/kit";
import { clockText, dayOf, dayText, whenText, zoneText } from "../zone";
import { calendarLinks, type GuestBooking, isLink, isTaken, priceText } from "./booking";
import { ClockSwitch, Closed, Frame, Loading, useDisplay } from "./Frame";
import { Pop } from "../Pop";
import { Picker } from "./Picker";
import * as site from "./site";

export function ManageRoute() {
  const { tenant = "", token = "" } = useParams();
  return <ManagePage tenant={tenant} token={token} />;
}

const HEAD: Record<GuestBooking["status"], { title: string; icon: ReactNode; tone: "green" | "amber" | "red" | "neutral" }> = {
  Confirmed: { title: "Your booking", icon: <CircleCheck size={22} />, tone: "green" },
  Pending: { title: "Waiting for confirmation", icon: <Hourglass size={22} />, tone: "amber" },
  Waitlisted: { title: "You are on the waitlist", icon: <Hourglass size={22} />, tone: "amber" },
  Cancelled: { title: "This booking was cancelled", icon: <CalendarX size={22} />, tone: "red" },
  Completed: { title: "This booking is done", icon: <CircleCheck size={22} />, tone: "neutral" },
  "No Show": { title: "This appointment was missed", icon: <CalendarX size={22} />, tone: "neutral" },
};

/** The ways to put a booking in a calendar. */
function AddToCalendar({ tenant, token, b, hour12 }: { tenant: string; token: string; b: GuestBooking; hour12: boolean }) {
  const start = Date.parse(b.start_time);
  const links = calendarLinks({
    title: `${b.service.name}${b.staff.name ? ` with ${b.staff.name}` : ""}`,
    start,
    end: Date.parse(b.end_time),
    where: b.location,
    details: `${whenText(start, b.time_zone, hour12)}${b.number ? `\nBooking ${b.number}` : ""}`,
  });
  return (
    <Pop
      className="gb-menu"
      summary={
        <>
          <CalendarPlus size={16} aria-hidden="true" /> Add to calendar
        </>
      }
    >
      <a href={site.calendarFile(tenant, token)} download>
        Apple Calendar or Outlook (.ics file)
      </a>
      <a href={links.google} target="_blank" rel="noopener noreferrer">
        Google Calendar
      </a>
      <a href={links.outlook} target="_blank" rel="noopener noreferrer">
        Outlook on the web
      </a>
    </Pop>
  );
}

function CancelDialog({ tenant, token, waitlisted, onClose, onDone }: { tenant: string; token: string; waitlisted: boolean; onClose: () => void; onDone: (b: GuestBooking) => void }) {
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      onDone((await site.cancel(tenant, token, reason)).booking);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };
  return (
    <Dialog
      open
      onClose={onClose}
      title={waitlisted ? "Leave the waitlist?" : "Cancel this booking?"}
      footer={
        <>
          <Button onClick={onClose}>{waitlisted ? "Stay on the waitlist" : "Keep my booking"}</Button>
          <Button variant="danger" loading={busy} onClick={() => void submit()}>
            {waitlisted ? "Leave the waitlist" : "Cancel booking"}
          </Button>
        </>
      }
    >
      <div className="stack">
        {error && <Alert tone="error">{error}</Alert>}
        <p className="muted">{waitlisted ? "You lose your place in line. You can join again whenever you like." : "The time is given up and the business is told. You can book again whenever you like."}</p>
        <Field label="Why are you cancelling? (optional)" htmlFor="gb-reason">
          <textarea id="gb-reason" className="textarea" rows={3} maxLength={1000} value={reason} onChange={(e) => setReason(e.target.value)} />
        </Field>
      </div>
    </Dialog>
  );
}

/** Choosing another time: the same month and times as a new booking, for this booking's own service. */
function Move({ tenant, token, b, onClose, onDone }: { tenant: string; token: string; b: GuestBooking; onClose: () => void; onDone: (b: GuestBooking) => void }) {
  const { hour12, setHour12 } = useDisplay();
  const zone = b.time_zone;
  const catalog = useQuery({ queryKey: ["gb-catalog", tenant], queryFn: ({ signal }) => site.catalog(tenant, signal), staleTime: 60_000 });
  const service = catalog.data?.services.find((s) => s.id === b.service.id);
  // The booking keeps its person unless the guest says anyone will do.
  const [staff, setStaff] = useState<string | null>(b.staff.id);
  const [day, setDay] = useState<string | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const qc = useQueryClient();
  const today = dayOf(Date.now(), zone);

  const confirm = async () => {
    if (!chosen) return;
    setBusy(true);
    setError(null);
    try {
      onDone((await site.move(tenant, token, chosen, staff ?? undefined)).booking);
    } catch (e) {
      setError(errorMessage(e));
      if (isTaken(e)) {
        setChosen(null);
        void qc.invalidateQueries({ queryKey: ["gb-slots"] });
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="gb-move" aria-label="Choose a new time">
      <div className="gb-stage-head">
        <h2 className="gb-h2">Choose a new time</h2>
        <ClockSwitch hour12={hour12} onChange={setHour12} />
      </div>
      <p className="muted">
        Now {whenText(Date.parse(b.start_time), zone, hour12)} · times shown in {zoneText(zone)}
      </p>
      {service && service.staff.length > 1 && (
        <fieldset className="gb-people">
          <legend className="field-label">With</legend>
          <div className="gb-chips">
            {[{ id: null, name: "Anyone free" }, ...service.staff].map((p) => (
              <label key={p.id ?? "any"} className="gb-chip" data-checked={staff === p.id || undefined}>
                <input
                  type="radio"
                  name="gb-move-person"
                  checked={staff === p.id}
                  onChange={() => {
                    setStaff(p.id);
                    setDay(null);
                    setChosen(null);
                  }}
                />
                {p.name}
              </label>
            ))}
          </div>
        </fieldset>
      )}
      <Picker
        scope={["move", tenant, token, staff ?? "any", zone]}
        load={(from, to, signal) => site.movableSlots(tenant, token, { from, to, zone, staff: staff ?? undefined, seats: b.seats }, signal)}
        zone={zone}
        hour12={hour12}
        today={today}
        day={day}
        at={chosen}
        showSeats={false}
        onDay={setDay}
        onPick={(s) => setChosen(s.start)}
      />
      {error && <Alert tone="error">{error}</Alert>}
      <div className="gb-submit">
        <p className="gb-total">{chosen ? <>Move to <strong>{whenText(Date.parse(chosen), zone, hour12)}</strong></> : "Pick a time to move to."}</p>
        <div className="row">
          <Button onClick={onClose}>Keep the current time</Button>
          <Button variant="primary" loading={busy} disabled={!chosen} onClick={() => void confirm()}>
            Confirm new time
          </Button>
        </div>
      </div>
    </section>
  );
}

export function ManagePage({ tenant, token }: { tenant: string; token: string }) {
  const state = useLocation().state as { booking?: GuestBooking; fresh?: boolean } | null;
  const qc = useQueryClient();
  const { hour12 } = useDisplay();
  const [mode, setMode] = useState<"view" | "move">("view");
  const [cancelling, setCancelling] = useState(false);
  const key = ["gb-mine", tenant, token];
  const q = useQuery({
    queryKey: key,
    queryFn: ({ signal }) => site.mine(tenant, token, signal).then((r) => r.booking),
    // Just booked: what the server answered is on screen already.
    initialData: state?.booking,
    staleTime: state?.booking ? 30_000 : 0,
    retry: false,
    refetchOnWindowFocus: true,
  });
  const b = q.data;
  // The price is written in the business's currency, which the booking itself does not carry.
  const catalog = useQuery({ queryKey: ["gb-catalog", tenant], queryFn: ({ signal }) => site.catalog(tenant, signal), staleTime: 60_000, retry: false });
  const home = `/book/${encodeURIComponent(tenant)}`;
  // Just booked: the page announces it.
  const fresh0 = !!state?.fresh;
  useEffect(() => {
    if (fresh0) document.getElementById("main")?.focus();
  }, [fresh0]);
  useShellTitle(b ? `${b.service.name} · ${dayText(dayOf(Date.parse(b.start_time), b.time_zone), { day: "numeric", month: "short" })}` : "Your booking");

  if (q.error) return <Frame><Closed error={q.error} what="link" home={home} onRetry={() => void q.refetch()} /></Frame>;
  if (!b) return <Frame><Loading label="Finding your booking" /></Frame>;

  const head = HEAD[b.status];
  const fresh = !!state?.fresh && (b.status === "Confirmed" || b.status === "Pending" || b.status === "Waitlisted");
  const start = Date.parse(b.start_time);
  const end = Date.parse(b.end_time);
  const active = b.status === "Confirmed" || b.status === "Pending";
  const until = (iso?: string | null) => (iso ? `${dayText(dayOf(Date.parse(iso), b.time_zone), { weekday: "short", day: "numeric", month: "short" })}, ${clockText(Date.parse(iso), b.time_zone, hour12)}` : "");
  const update = (next: GuestBooking) => qc.setQueryData(key, next);

  if (mode === "move") {
    return (
      <Frame wide>
        <Move
          tenant={tenant}
          token={token}
          b={b}
          onClose={() => setMode("view")}
          onDone={(next) => {
            update(next);
            setMode("view");
            toast("Your booking has been moved");
          }}
        />
      </Frame>
    );
  }

  return (
    <Frame>
      <article className="gb-booking" data-status={b.status}>
        <header className="gb-booking-head">
          <span className="gb-status-icon" data-tone={head.tone} aria-hidden="true">
            {head.icon}
          </span>
          <div>
            <h1 className="gb-h1">{fresh ? (b.status === "Confirmed" ? "You are booked" : head.title) : head.title}</h1>
            {fresh && (
              <p className="muted">
                <Mail size={14} aria-hidden="true" /> {b.status === "Pending" ? "We will email you as soon as it is confirmed." : `We are emailing the details to ${b.guest.email ?? "you"}.`}
              </p>
            )}
            {b.status === "Pending" && !fresh && <p className="muted">The business will confirm it by email. The time is held for you meanwhile.</p>}
            {b.status === "Waitlisted" && (
              <p className="muted">
                {b.waitlist_position ? `You are number ${b.waitlist_position} in line. ` : ""}We will email you if a seat becomes free.
              </p>
            )}
            {b.status === "Cancelled" && b.cancellation_reason && <p className="muted">Reason given: {b.cancellation_reason}</p>}
          </div>
        </header>

        <div className="gb-card">
          <div className="gb-card-main">
            <strong className="gb-what">{b.service.name}</strong>
            <Badge tone={head.tone}>{b.status === "No Show" ? "Missed" : b.status}</Badge>
          </div>
          <dl className="gb-details">
            <div>
              <dt>When</dt>
              <dd>
                <strong>{dayText(dayOf(start, b.time_zone), { weekday: "long", day: "numeric", month: "long", year: "numeric" })}</strong>
                <br />
                {clockText(start, b.time_zone, hour12)} – {clockText(end, b.time_zone, hour12)} <span className="faint">({zoneText(b.time_zone, start)})</span>
              </dd>
            </div>
            {b.staff.name && (
              <div>
                <dt>With</dt>
                <dd>{b.staff.name}</dd>
              </div>
            )}
            {b.status !== "Cancelled" && (
              <div>
                <dt>Where</dt>
                <dd>
                  {b.location_type === "Video" ? "Video call" : b.location_type === "Phone" ? "Phone call" : b.location ? null : "In person"}
                  {b.location &&
                    (isLink(b.location) ? (
                      <a href={b.location} target="_blank" rel="noopener noreferrer">
                        {b.location}
                      </a>
                    ) : (
                      <span className="gb-line">{b.location}</span>
                    ))}
                </dd>
              </div>
            )}
            {b.seats > 1 && (
              <div>
                <dt>Seats</dt>
                <dd>{b.seats}</dd>
              </div>
            )}
            {Number(b.amount) > 0 && (
              <div>
                <dt>Price</dt>
                <dd>{priceText(b.amount, catalog.data?.currency ?? null)}</dd>
              </div>
            )}
            <div>
              <dt>Booked by</dt>
              <dd>
                {[b.guest.name, b.guest.email, b.guest.phone].filter(Boolean).map((v) => (
                  <span key={v} className="gb-line">
                    {v}
                  </span>
                ))}
              </dd>
            </div>
            {b.answers.map((a) => (
              <div key={a.question}>
                <dt>{a.question}</dt>
                <dd>{a.answer}</dd>
              </div>
            ))}
            {b.number && (
              <div>
                <dt>Reference</dt>
                <dd>{b.number}</dd>
              </div>
            )}
          </dl>
        </div>

        {(active || b.status === "Waitlisted") && (
          <div className="gb-actions">
            {active && <AddToCalendar tenant={tenant} token={token} b={b} hour12={hour12} />}
            {b.can_reschedule && (
              <Button onClick={() => setMode("move")}>Choose another time</Button>
            )}
            {b.can_cancel && (
              <Button variant="danger" onClick={() => setCancelling(true)}>
                {b.status === "Waitlisted" ? "Leave the waitlist" : "Cancel booking"}
              </Button>
            )}
          </div>
        )}
        {active && (
          <p className="gb-windows muted">
            {b.can_reschedule || b.can_cancel ? (
              <>
                {b.can_reschedule && <>You can move it until {until(b.reschedule_until)}. </>}
                {b.can_cancel && <>You can cancel it until {until(b.cancel_until)}.</>}
              </>
            ) : (
              "It can no longer be changed online. Please get in touch with the business if your plans have changed."
            )}
          </p>
        )}
        {b.status === "Cancelled" && (
          <div className="gb-actions">
            <Link className="btn btn-primary btn-md" to={`${home}?service=${b.service.id}`}>
              Book another time
            </Link>
          </div>
        )}
        <p className="gb-keep faint">Keep this link: it is how you come back to this booking.</p>
      </article>
      {cancelling && (
        <CancelDialog
          tenant={tenant}
          token={token}
          waitlisted={b.status === "Waitlisted"}
          onClose={() => setCancelling(false)}
          onDone={(next) => {
            update(next);
            setCancelling(false);
            toast(next.status === "Cancelled" ? "Your booking has been cancelled" : "Done");
          }}
        />
      )}
    </Frame>
  );
}

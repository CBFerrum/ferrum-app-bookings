// A guest's own booking (/bookings/book/{workspace}/manage/{token}, the link in their email): what
// was booked and where, a way to put it in a calendar, and, while the business still allows it, a
// move to another time or a cancellation. The page shows what the server says (its windows, its
// refusals); the link is the guest's only key, so it names nothing but their booking.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { CalendarPlus, CircleCheck, Hourglass, CalendarX, Mail } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { Link, useLocation, useParams } from "react-router";
import { errorMessage, t, isolate } from "@ferrum/client";
import { Alert, Badge, Button, Dialog, Field, toast, useShellTitle, placed } from "@ferrum/kit";
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
  Confirmed: { get title() { return t("Your booking"); }, icon: <CircleCheck size={22} />, tone: "green" },
  Pending: { get title() { return t("Waiting for confirmation"); }, icon: <Hourglass size={22} />, tone: "amber" },
  Waitlisted: { get title() { return t("You are on the waitlist"); }, icon: <Hourglass size={22} />, tone: "amber" },
  Cancelled: { get title() { return t("This booking was cancelled"); }, icon: <CalendarX size={22} />, tone: "red" },
  Completed: { get title() { return t("This booking is done"); }, icon: <CircleCheck size={22} />, tone: "neutral" },
  "No Show": { get title() { return t("This appointment was missed"); }, icon: <CalendarX size={22} />, tone: "neutral" },
};

/** The ways to put a booking in a calendar. */
function AddToCalendar({ tenant, token, b, hour12 }: { tenant: string; token: string; b: GuestBooking; hour12: boolean }) {
  const start = Date.parse(b.start_time);
  const links = calendarLinks({
    title: `${b.service.name}${b.staff.name ? t(" with {name}", { name: b.staff.name }) : ""}`,
    start,
    end: Date.parse(b.end_time),
    where: b.location,
    details: `${whenText(start, b.time_zone, hour12)}${b.number ? t("\nBooking {number}", { number: b.number }) : ""}`,
  });
  return (
    <Pop
      className="gb-menu"
      summary={
        <>
          <CalendarPlus size={16} aria-hidden="true" /> {t("Add to calendar")}
        </>
      }
    >
      <a href={site.calendarFile(tenant, token)} download>
        {t("Apple Calendar or Outlook (.ics file)")}
      </a>
      <a href={links.google} target="_blank" rel="noopener noreferrer">
        {t("Google Calendar")}
      </a>
      <a href={links.outlook} target="_blank" rel="noopener noreferrer">
        {t("Outlook on the web")}
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
      title={waitlisted ? t("Leave the waitlist?") : t("Cancel this booking?")}
      footer={
        <>
          <Button onClick={onClose}>{waitlisted ? t("Stay on the waitlist") : t("Keep my booking")}</Button>
          <Button variant="danger" loading={busy} onClick={() => void submit()}>
            {waitlisted ? t("Leave the waitlist") : t("Cancel booking")}
          </Button>
        </>
      }
    >
      <div className="stack">
        {error && <Alert tone="error">{isolate(error)}</Alert>}
        <p className="muted">{waitlisted ? t("You lose your place in line. You can join again whenever you like.") : t("The time is given up and the business is told. You can book again whenever you like.")}</p>
        <Field label={t("Why are you cancelling? (optional)")} htmlFor="gb-reason">
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
      const moved = (await site.move(tenant, token, chosen, staff ?? undefined)).booking;
      void qc.invalidateQueries({ queryKey: ["gb-slots"] });
      onDone(moved);
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
    <section className="gb-move" aria-label={t("Choose a new time")}>
      <div className="gb-stage-head">
        <h2 className="gb-h2">{t("Choose a new time")}</h2>
        <ClockSwitch hour12={hour12} onChange={setHour12} />
      </div>
      <p className="muted">
        {placed(t("Now {p1} · times shown in {p2}"), { p1: isolate(whenText(Date.parse(b.start_time), zone, hour12)), p2: isolate(zoneText(zone)) })}
      </p>
      {service && service.staff.length > 1 && (
        <fieldset className="gb-people">
          <legend className="field-label">{t("With")}</legend>
          <div className="gb-chips">
            {[{ id: null, name: t("Anyone free") }, ...service.staff].map((p) => (
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
                {isolate(p.name)}
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
      {error && <Alert tone="error">{isolate(error)}</Alert>}
      <div className="gb-submit">
        <p className="gb-total">{chosen ? <>{t("Move to")} <strong>{isolate(whenText(Date.parse(chosen), zone, hour12))}</strong></> : t("Pick a time to move to.")}</p>
        <div className="row">
          <Button onClick={onClose}>{t("Keep the current time")}</Button>
          <Button variant="primary" loading={busy} disabled={!chosen} onClick={() => void confirm()}>
            {t("Confirm new time")}
          </Button>
        </div>
      </div>
    </section>
  );
}

export function ManagePage({ tenant, token }: { tenant: string; token: string }) {
  const state = useLocation().state as { booking?: GuestBooking; fresh?: boolean; later?: { id: string; start_time: string }[] } | null;
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
  useShellTitle(b ? `${b.service.name} · ${dayText(dayOf(Date.parse(b.start_time), b.time_zone), { day: "numeric", month: "short" })}` : t("Your booking"));

  if (q.error) return <Frame><Closed error={q.error} what="link" home={home} onRetry={() => void q.refetch()} /></Frame>;
  if (!b) return <Frame><Loading label={t("Finding your booking")} /></Frame>;

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
            toast(t("Your booking has been moved"));
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
            <h1 className="gb-h1">{fresh ? (b.status === "Confirmed" ? t("You are booked") : head.title) : head.title}</h1>
            {fresh && (
              <p className="muted">
                <Mail size={14} aria-hidden="true" /> {b.status === "Pending" ? t("We will email you as soon as it is confirmed.") : t("We are emailing the details to {value}.", { value: b.guest.email ?? "you" })}
              </p>
            )}
            {fresh && (state?.later?.length ?? 0) > 0 && (
              <div className="muted">
                <p>{t("Also booked, at the same time:")}</p>
                <ul>
                  {state?.later?.map((o) => (
                    <li key={o.id}>{isolate(until(o.start_time))}</li>
                  ))}
                </ul>
              </div>
            )}
            {b.status === "Pending" && !fresh && <p className="muted">{t("The business will confirm it by email. The time is held for you meanwhile.")}</p>}
            {b.status === "Waitlisted" && (
              <p className="muted">
                {(b.waitlist_position ? t("You are number {waitlist_position} in line. We will email you if a seat becomes free.", { waitlist_position: b.waitlist_position }) : t("We will email you if a seat becomes free."))}
              </p>
            )}
            {b.status === "Cancelled" && b.cancellation_reason && <p className="muted">{t("Reason given: {cancellation_reason}", { cancellation_reason: b.cancellation_reason })}</p>}
          </div>
        </header>

        <div className="gb-card">
          <div className="gb-card-main">
            <strong className="gb-what">{isolate(b.service.name)}</strong>
            <Badge tone={head.tone}>{b.status === "No Show" ? t("Missed") : b.status}</Badge>
          </div>
          <dl className="gb-details">
            <div>
              <dt>{t("When")}</dt>
              <dd>
                <strong>{isolate(dayText(dayOf(start, b.time_zone), { weekday: "long", day: "numeric", month: "long", year: "numeric" }))}</strong>
                <br />
                {isolate(clockText(start, b.time_zone, hour12))} – {isolate(clockText(end, b.time_zone, hour12))} <span className="faint">({isolate(zoneText(b.time_zone, start))})</span>
              </dd>
            </div>
            {b.staff.name && (
              <div>
                <dt>{t("With")}</dt>
                <dd>{isolate(b.staff.name)}</dd>
              </div>
            )}
            {b.status !== "Cancelled" && (
              <div>
                <dt>{t("Where")}</dt>
                <dd>
                  {b.location_type === "Video" ? t("Video call") : b.location_type === "Phone" ? t("Phone call") : b.location ? null : t("In person")}
                  {b.location &&
                    (isLink(b.location) ? (
                      <a href={b.location} target="_blank" rel="noopener noreferrer">
                        {isolate(b.location)}
                      </a>
                    ) : (
                      <span className="gb-line">{b.location}</span>
                    ))}
                </dd>
              </div>
            )}
            {b.seats > 1 && (
              <div>
                <dt>{t("Seats")}</dt>
                <dd>{b.seats}</dd>
              </div>
            )}
            {Number(b.amount) > 0 && (
              <div>
                <dt>{t("Price")}</dt>
                <dd>{isolate(priceText(b.amount, catalog.data?.currency ?? null))}</dd>
              </div>
            )}
            <div>
              <dt>{t("Booked by")}</dt>
              <dd>
                {[b.guest.name, b.guest.email, b.guest.phone].filter(Boolean).map((v) => (
                  <span key={v} className="gb-line">
                    {isolate(v ?? "")}
                  </span>
                ))}
              </dd>
            </div>
            {b.answers.map((a) => (
              <div key={a.question}>
                <dt>{isolate(a.question)}</dt>
                <dd>{isolate(a.answer)}</dd>
              </div>
            ))}
            {b.number && (
              <div>
                <dt>{t("Reference")}</dt>
                <dd>{isolate(b.number)}</dd>
              </div>
            )}
          </dl>
        </div>

        {active && b.clash && (
          <p className="gb-windows" role="status">
            {t("Your host is no longer free at this time.")}{" "}
            {b.can_reschedule ? t("You can choose another time.") : t("The business will be in touch about another time.")}
          </p>
        )}
        {(active || b.status === "Waitlisted") && (
          <div className="gb-actions">
            {active && <AddToCalendar tenant={tenant} token={token} b={b} hour12={hour12} />}
            {b.can_reschedule && (
              <Button onClick={() => setMode("move")}>{t("Choose another time")}</Button>
            )}
            {b.can_cancel && (
              <Button variant="danger" onClick={() => setCancelling(true)}>
                {b.status === "Waitlisted" ? t("Leave the waitlist") : t("Cancel booking")}
              </Button>
            )}
          </div>
        )}
        {active && (
          <p className="gb-windows muted">
            {b.can_reschedule || b.can_cancel ? (
              <>
                {b.can_reschedule && <>{t("You can move it until")} {isolate(until(b.reschedule_until))}. </>}
                {b.can_cancel && <>{t("You can cancel it until")} {isolate(until(b.cancel_until))}.</>}
              </>
            ) : (
              t("It can no longer be changed online. Please get in touch with the business if your plans have changed.")
            )}
          </p>
        )}
        {b.status === "Cancelled" && (
          <div className="gb-actions">
            <Link className="btn btn-primary btn-md" to={`${home}?service=${b.service.id}`}>
              {t("Book another time")}
            </Link>
          </div>
        )}
        <p className="gb-keep faint">{t("Keep this link: it is how you come back to this booking.")}</p>
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
            toast(next.status === "Cancelled" ? t("Your booking has been cancelled") : t("Done"));
          }}
        />
      )}
    </Frame>
  );
}

// The public booking page (a `shell.public` page: no sign-in, no bar) at /bookings/book/{workspace}:
// choose a service, a person when it matters, a day and a time from what is really free in the
// guest's own time zone, say who they are, and book. Every step is in the address, so Back goes back
// a step and a link can start anywhere (?service=…). The server decides everything: a time taken
// while the guest typed disables Confirm with a message, and its complaints land beside the field.
// A booking ends on its manage page, where the guest can add it to a calendar, move it or cancel it.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Banknote, ChevronLeft, Clock, Globe, MapPin, Users } from "lucide-react";
import { type FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { ApiError, bootstrap, t, isolate } from "@ferrum/client";
import { Alert, Button, EmptyState, Field, useShellTitle } from "@ferrum/kit";
import { dayOf, isDate, whenText, zoneList, zoneText } from "../zone";
import { type Catalog, check, bookBody, durationText, type Form, hasErrors, isTaken, noErrors, priceText, type Question, type Service, serverErrors, whereText } from "./booking";
import { ClockSwitch, Closed, Frame, Loading, useDisplay } from "./Frame";
import { ManagePage } from "./Manage";
import { Picker } from "./Picker";
import * as site from "./site";

export function BookPage() {
  const { tenant = "" } = useParams();
  const [sp] = useSearchParams();
  // A guest's own link, when the business pointed its manage address at this page (?token=…).
  const token = sp.get("token");
  return token ? <ManagePage tenant={tenant} token={token} /> : <Booker tenant={tenant} />;
}

function Facts({ service, currency }: { service: Service; currency: string | null }) {
  return (
    <ul className="gb-facts">
      <li>
        <Clock size={15} aria-hidden="true" /> {isolate(durationText(service.duration_minutes))}
      </li>
      <li>
        <MapPin size={15} aria-hidden="true" /> {isolate(whereText(service.location_type, service.location))}
      </li>
      <li>
        <Banknote size={15} aria-hidden="true" /> {isolate(priceText(service.price, currency))}
        {service.seats > 1 && Number(service.price) > 0 ? t(" a seat") : ""}
      </li>
      {service.seats > 1 && (
        <li>
          <Users size={15} aria-hidden="true" /> {t("Group class · up to {seats} seats", { seats: service.seats })}
        </li>
      )}
    </ul>
  );
}

function ServiceList({ catalog }: { catalog: Catalog }) {
  const name = bootstrap()?.workspace.name;
  return (
    <div className="gb-services">
      <h1 className="gb-h1">{name ? t("Book with {name}", { name }) : t("Book an appointment")}</h1>
      <p className="muted">{t("What would you like to book?")}</p>
      <ul className="gb-service-list">
        {catalog.services.map((s) => (
          <li key={s.id}>
            <Link className="gb-service-card" to={{ search: `?service=${s.id}` }}>
              <strong>{isolate(s.name)}</strong>
              <Facts service={s} currency={catalog.currency} />
              {s.description && <p className="gb-clamp">{isolate(s.description)}</p>}
              {s.staff.length > 1 && <span className="faint">{t("With")} {isolate(s.staff.map((p) => p.name).join(", "))}</span>}
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Who the guest sees, when more than one person gives the service. */
function People({ service, value, onChange }: { service: Service; value: string | null; onChange: (id: string | null) => void }) {
  if (service.staff.length < 2) return null;
  const options = [{ id: null, name: "Anyone free" }, ...service.staff];
  return (
    <fieldset className="gb-people">
      <legend className="field-label">{t("Who would you like to see?")}</legend>
      <div className="gb-chips">
        {options.map((p) => (
          <label key={p.id ?? "any"} className="gb-chip" data-checked={value === p.id || undefined}>
            <input type="radio" name="gb-person" checked={value === p.id} onChange={() => onChange(p.id)} />
            {isolate(p.name)}
          </label>
        ))}
      </div>
    </fieldset>
  );
}

function Seats({ max, value, onChange }: { max: number; value: number; onChange: (n: number) => void }) {
  if (max < 2) return null;
  return (
    <div className="gb-seats">
      <span className="field-label" id="gb-seats-label">
        {t("Seats")}
      </span>
      <div className="gb-stepper" role="group" aria-labelledby="gb-seats-label">
        <button type="button" className="icon-btn" aria-label={t("One seat fewer")} disabled={value <= 1} onClick={() => onChange(value - 1)}>
          −
        </button>
        <output aria-live="polite">{value}</output>
        <button type="button" className="icon-btn" aria-label={t("One seat more")} disabled={value >= max} onClick={() => onChange(value + 1)}>
          +
        </button>
      </div>
    </div>
  );
}

/** The zone times are shown in. The list stays plain names: an offset for each of four hundred
 *  zones is work nobody needs until they have chosen one. */
function ZoneSelect({ zone, workspace, onChange }: { zone: string; workspace: string; onChange: (z: string) => void }) {
  const zones = useMemo(() => zoneList([zone, workspace]), [zone, workspace]);
  return (
    <label className="gb-zone" title={zoneText(zone)}>
      <Globe size={14} aria-hidden="true" />
      <span className="sr-only">{t("Time zone")}</span>
      <select className="select" value={zone} onChange={(e) => onChange(e.target.value)}>
        {zones.map((z) => (
          <option key={z} value={z}>
            {isolate(z.replace(/_/g, " "))}
          </option>
        ))}
      </select>
    </label>
  );
}

function Booker({ tenant }: { tenant: string }) {
  const [sp, setSp] = useSearchParams();
  const { zone, setZone, hour12, setHour12 } = useDisplay();
  const cat = useQuery({ queryKey: ["gb-catalog", tenant], queryFn: ({ signal }) => site.catalog(tenant, signal), staleTime: 60_000, retry: false });
  const catalog = cat.data;
  const only = catalog?.services.length === 1 ? catalog.services[0] : undefined;
  const service = catalog?.services.find((s) => s.id === sp.get("service")) ?? only;
  const choice = sp.get("with");
  const staff = service?.staff.some((p) => p.id === choice) ? choice : null;
  const seats = Math.min(Math.max(Number(sp.get("seats")) || 1, 1), service?.seats ?? 1);
  const dayParam = sp.get("day");
  const day = isDate(dayParam) ? dayParam : null;
  const atParam = sp.get("at");
  const at = atParam && Number.isFinite(Date.parse(atParam)) ? atParam : null;
  const waitlist = sp.get("wait") === "1";
  const today = dayOf(Date.now(), zone);
  const workspaceName = bootstrap()?.workspace.name;
  useShellTitle(service ? `${service.name}${workspaceName ? ` · ${workspaceName}` : ""}` : workspaceName ? t("Book with {workspaceName}", { workspaceName }) : t("Book an appointment"));

  const go = (patch: Record<string, string | null>, replace = false) => {
    const next = new URLSearchParams(sp);
    for (const [k, v] of Object.entries(patch)) {
      if (v === null) next.delete(k);
      else next.set(k, v);
    }
    setSp(next, { replace });
  };

  if (cat.error) return <Frame><Closed error={cat.error} what="page" onRetry={() => void cat.refetch()} /></Frame>;
  if (!catalog) return <Frame><Loading label={t("Loading what you can book")} /></Frame>;
  if (!catalog.services.length) {
    return (
      <Frame>
        <EmptyState title={t("Nothing to book yet")} icon="calendar">
          <p className="muted">{t("There are no appointments open for booking at the moment. Please check back soon.")}</p>
        </EmptyState>
      </Frame>
    );
  }
  if (!service) {
    return (
      <Frame>
        <ServiceList catalog={catalog} />
      </Frame>
    );
  }

  return (
    <Frame wide>
      <div className="gb-booker" data-step={at ? "details" : "times"}>
        <aside className="gb-panel" aria-label={t("About this booking")}>
          {catalog.services.length > 1 && (
            <Link className="gb-back" to={{ search: "" }}>
              <ChevronLeft size={15} aria-hidden="true" /> {t("All services")}
            </Link>
          )}
          <h1 className="gb-h1">{isolate(service.name)}</h1>
          <Facts service={service} currency={catalog.currency} />
          {service.description && <p className="gb-about">{isolate(service.description)}</p>}
          {at ? (
            <div className="gb-when">
              <strong>{isolate(whenText(Date.parse(at), zone, hour12))}</strong>
              <span className="faint">{isolate(zoneText(zone))}</span>
              <button type="button" className="link-btn" onClick={() => go({ at: null, wait: null })}>
                {t("Choose another time")}
              </button>
            </div>
          ) : (
            <>
              <People service={service} value={staff} onChange={(id) => go({ with: id, day: null, at: null })} />
              <Seats max={service.seats} value={seats} onChange={(n) => go({ seats: n === 1 ? null : String(n), at: null })} />
            </>
          )}
        </aside>
        <section className="gb-stage" aria-label={at ? t("Your details") : t("Choose a time")}>
          {at ? (
            <Details tenant={tenant} service={service} currency={catalog.currency} at={at} staff={staff} seats={seats} zone={zone} waitlist={waitlist} onAnother={() => go({ at: null, wait: null })} />
          ) : (
            <>
              <div className="gb-stage-head">
                <h2 className="gb-h2">{t("Choose a time")}</h2>
                <div className="gb-stage-tools">
                  <ZoneSelect zone={zone} workspace={catalog.time_zone} onChange={(z) => { setZone(z); go({ day: null, at: null }); }} />
                  <ClockSwitch hour12={hour12} onChange={setHour12} />
                </div>
              </div>
              <Picker
                scope={[tenant, service.id, staff ?? "any", seats, zone]}
                load={(from, to, signal) => site.freeSlots(tenant, service.id, { from, to, zone, staff: staff ?? undefined, seats }, signal)}
                zone={zone}
                hour12={hour12}
                today={today}
                day={day}
                at={at}
                showSeats={service.seats > 1}
                onDay={(d) => go({ day: d }, true)}
                onPick={(s) => go({ at: s.start, day: dayOf(Date.parse(s.start), zone), wait: s.full ? "1" : null })}
              />
            </>
          )}
        </section>
      </div>
    </Frame>
  );
}

// ── the form ──

function Answer({ q, value, error, onChange }: { q: Question; value: string; error?: string; onChange: (v: string) => void }) {
  const id = `gb-q-${q.question.replace(/\W+/g, "-").toLowerCase()}`;
  const label = q.required ? q.question : t("{question} (optional)", { question: q.question });
  const invalid = { "aria-invalid": error ? true : undefined, "aria-describedby": error ? `${id}-error` : undefined } as const;
  if (q.type === "Yes/No" || (q.type === "Choice" && q.options.length <= 4)) {
    const options = q.type === "Yes/No" ? ["Yes", "No"] : q.options;
    return (
      <fieldset className={`field gb-radios ${error ? "field-invalid" : ""}`}>
        <legend className="field-label">
          {isolate(label)}
          {q.required && <span className="req" aria-hidden="true">*</span>}
        </legend>
        <div className="gb-chips">
          {options.map((o) => (
            <label key={o} className="gb-chip" data-checked={value === o || undefined}>
              <input type="radio" name={id} checked={value === o} onChange={() => onChange(o)} {...(error ? { "aria-describedby": `${id}-error` } : {})} />
              {isolate(o)}
            </label>
          ))}
        </div>
        {error && <p className="field-error" id={`${id}-error`}>{isolate(error)}</p>}
      </fieldset>
    );
  }
  return (
    <Field label={label} htmlFor={id} required={q.required} error={error}>
      {q.type === "Long Text" ? (
        <textarea id={id} className="textarea" rows={3} maxLength={2000} value={value} onChange={(e) => onChange(e.target.value)} {...invalid} />
      ) : q.type === "Choice" ? (
        <select id={id} className="select" value={value} onChange={(e) => onChange(e.target.value)} {...invalid}>
          <option value="">{t("Choose…")}</option>
          {q.options.map((o) => (
            <option key={o}>{isolate(o)}</option>
          ))}
        </select>
      ) : (
        <input id={id} className="input" inputMode={q.type === "Number" ? "decimal" : undefined} maxLength={500} value={value} onChange={(e) => onChange(e.target.value)} {...invalid} />
      )}
    </Field>
  );
}

function Details({ tenant, service, currency, at, staff, seats, zone, waitlist, onAnother }: {
  tenant: string;
  service: Service;
  currency: string | null;
  at: string;
  staff: string | null;
  seats: number;
  zone: string;
  /** The class is full: this joins its waitlist. */
  waitlist: boolean;
  onAnother: () => void;
}) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const form = useRef<HTMLFormElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const [values, setValues] = useState<Form>({ name: "", email: "", phone: "", notes: "", seats, answers: {} });
  const [errors, setErrors] = useState(noErrors);
  const [busy, setBusy] = useState(false);
  const [trap, setTrap] = useState("");
  const [taken, setTaken] = useState(false);
  const [received, setReceived] = useState(false);
  const day = dayOf(Date.parse(at), zone);

  // Is the time still free? Asked again when the guest comes back to the tab, and every minute.
  const still = useQuery({
    queryKey: ["gb-still", tenant, service.id, staff ?? "any", seats, zone, at],
    queryFn: ({ signal }) => site.freeSlots(tenant, service.id, { from: day, to: day, zone, staff: staff ?? undefined, seats }, signal),
    refetchInterval: 60_000,
    refetchOnWindowFocus: true,
    staleTime: 0,
  });
  const gone = taken || (!!still.data && !still.data.days.some((d) => d.slots.some((s) => s.start === at)));
  useEffect(() => setTaken(false), [at]);
  // The form replaced the times: the focus goes with it.
  useEffect(() => heading.current?.focus(), []);

  const set = (patch: Partial<Form>) => setValues((v) => ({ ...v, ...patch }));
  const firstProblem = () => requestAnimationFrame(() => form.current?.querySelector<HTMLElement>('[aria-invalid="true"], .field-invalid input, .field-invalid select')?.focus());

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const found = check(values, service.questions);
    setErrors(found);
    if (hasErrors(found)) return firstProblem();
    setBusy(true);
    try {
      const out = await site.book(tenant, bookBody({ service, start: at, staff: staff ?? undefined, form: values, zone, waitlist, website: trap }));
      // That time is no longer free for the next guest, whatever the lists on screen last said.
      void qc.invalidateQueries({ queryKey: ["gb-slots"] });
      if ("status" in out) setReceived(true);
      else navigate(`manage/${out.token}`, { replace: true, state: { booking: out.booking, fresh: true } });
    } catch (err) {
      if (isTaken(err)) {
        setTaken(true);
        void still.refetch();
        void qc.invalidateQueries({ queryKey: ["gb-slots"] });
      }
      const found = serverErrors(err, service.questions);
      // A connection that dropped may have booked it: say so, rather than invite a second try.
      if (err instanceof ApiError && err.status === 0) found.general = ["We could not reach the server, so we cannot tell whether your booking went through. Please check your email for a confirmation before trying again."];
      setErrors(found);
      firstProblem();
    } finally {
      setBusy(false);
    }
  };

  if (received) {
    return (
      <EmptyState title={t("Thank you")} icon="check">
        <p className="muted">{t("We have your request.")}</p>
      </EmptyState>
    );
  }
  const err = (k: "name" | "email" | "phone" | "notes") => errors[k];
  const invalid = (k: "name" | "email" | "phone" | "notes") => ({ "aria-invalid": err(k) ? true : undefined, "aria-describedby": err(k) ? `gb-${k}-error` : undefined }) as const;
  return (
    <form ref={form} className="gb-form" onSubmit={(e) => void submit(e)} noValidate>
      <h2 className="gb-h2" ref={heading} tabIndex={-1}>
        {waitlist ? t("Join the waitlist") : t("Your details")}
      </h2>
      {waitlist && !gone && <Alert tone="info">{t("This class is full. Join the waitlist and we will email you if a seat becomes free.")}</Alert>}
      {gone && (
        <Alert tone="warning" action={<Button size="sm" onClick={onAnother}>{t("Choose another time")}</Button>}>
          {taken ? t("Someone has just taken that time.") : t("That time is no longer free.")} {t("Please choose another.")}
        </Alert>
      )}
      {errors.general.length > 0 && (
        <Alert tone="error">
          {errors.general.map((m) => (
            <p key={m}>{isolate(m)}</p>
          ))}
        </Alert>
      )}
      <div className="gb-form-grid">
        <Field label={t("Your name")} htmlFor="gb-name" required error={errors.name}>
          <input id="gb-name" className="input" autoComplete="name" maxLength={200} value={values.name} onChange={(e) => set({ name: e.target.value })} {...invalid("name")} />
        </Field>
        <Field label={t("Email")} htmlFor="gb-email" required error={errors.email} hint={t("Your confirmation and the link to change your booking go here.")}>
          <input id="gb-email" className="input" type="email" inputMode="email" autoComplete="email" maxLength={254} value={values.email} onChange={(e) => set({ email: e.target.value })} {...invalid("email")} />
        </Field>
        <Field label={t("Phone (optional)")} htmlFor="gb-phone" error={errors.phone}>
          <input id="gb-phone" className="input" type="tel" inputMode="tel" autoComplete="tel" maxLength={40} value={values.phone} onChange={(e) => set({ phone: e.target.value })} {...invalid("phone")} />
        </Field>
      </div>
      {service.questions.map((q) => (
        <Answer key={q.question} q={q} value={values.answers[q.question] ?? ""} error={errors.answers[q.question]} onChange={(v) => set({ answers: { ...values.answers, [q.question]: v } })} />
      ))}
      <Field label={t("Anything we should know? (optional)")} htmlFor="gb-notes" error={errors.notes}>
        <textarea id="gb-notes" className="textarea" rows={3} maxLength={2000} value={values.notes} onChange={(e) => set({ notes: e.target.value })} {...invalid("notes")} />
      </Field>
      {/* A field people never see: whatever fills it is a program. */}
      <div className="gb-trap" aria-hidden="true">
        <label>
          {t("Website")}
          <input name="website" tabIndex={-1} autoComplete="off" value={trap} onChange={(e) => setTrap(e.target.value)} />
        </label>
      </div>
      <div className="gb-submit">
        <p className="gb-total">
          {isolate(service.name)}
          {seats > 1 ? t(" · {seats} seats", { seats }) : ""} · <strong>{isolate(priceText(Number(service.price) * seats, currency))}</strong>
        </p>
        <Button type="submit" variant="primary" loading={busy} disabled={gone}>
          {waitlist ? t("Join the waitlist") : t("Confirm booking")}
        </Button>
      </div>
    </form>
  );
}


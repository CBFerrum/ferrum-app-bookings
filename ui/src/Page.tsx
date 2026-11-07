// The booking page, from the team's side: whether guests can book online, the address to share, and
// the few rules people change most (confirm by hand, notice, how far ahead, when a guest can still
// cancel or move, reminders). Each setting saves where it stands, against the version on screen; the
// server checks it. Everything else about the app's settings is one link away.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy, ExternalLink } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router";
import { api, bootstrap, errorMessage, routeBase, t, isolate } from "@ferrum/client";
import { Badge, Button, EmptyState, ErrorState, PageLoading, toast, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { type Doc, type EntityMeta, useEntityMeta, useRecord } from "@ferrum/records";
import { rows } from "./data";
import { Tabs } from "./Tabs";

type Save = (name: string, value: unknown) => Promise<void>;

/** A yes or no that takes effect when it is pressed. */
function Switch({ id, label, hint, checked, disabled, onChange }: { id: string; label: string; hint?: string; checked: boolean; disabled?: boolean; onChange: (v: boolean) => void }) {
  return (
    <div className="bk-setting">
      <div className="bk-setting-text">
        <label htmlFor={id}>{isolate(label)}</label>
        {hint && <p className="faint">{isolate(hint)}</p>}
      </div>
      <input id={id} className="bk-switch" type="checkbox" role="switch" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
    </div>
  );
}

/** A whole number that is saved when the field is left (or Enter is pressed). */
function Count({ id, label, hint, value, min, max, unit, disabled, onSave }: { id: string; label: string; hint?: string; value: number; min: number; max: number; unit: string; disabled?: boolean; onSave: (n: number) => void }) {
  const [text, setText] = useState(String(value));
  const [bad, setBad] = useState(false);
  useEffect(() => setText(String(value)), [value]);
  const commit = () => {
    const n = Number(text);
    if (text.trim() === "" || !Number.isInteger(n) || n < min || n > max) {
      setBad(true);
      return;
    }
    setBad(false);
    if (n !== value) onSave(n);
  };
  return (
    <div className="bk-setting">
      <div className="bk-setting-text">
        <label htmlFor={id}>{isolate(label)}</label>
        <p className={bad ? "field-error" : "faint"} id={`${id}-hint`}>
          {bad ? t("Enter a whole number from {min} to {max}.", { min, max }) : hint}
        </p>
      </div>
      <span className="bk-count">
        <input
          id={id}
          className="input"
          inputMode="numeric"
          value={text}
          disabled={disabled}
          aria-invalid={bad || undefined}
          aria-describedby={`${id}-hint`}
          onChange={(e) => setText(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && commit()}
        />
        <span className="muted">{isolate(unit)}</span>
      </span>
    </div>
  );
}

const REMIND = [15, 30, 60, 120, 240, 720, 1440, 2880, 10080];

const before = (m: number) =>
  m % 10080 === 0 ? `${m / 10080} week${m === 10080 ? "" : "s"} before` : m % 1440 === 0 ? `${m / 1440} day${m === 1440 ? "" : "s"} before` : m % 60 === 0 ? `${m / 60} hour${m === 60 ? "" : "s"} before` : `${m} minutes before`;

function Reminders({ doc, save, writable }: { doc: Doc; save: Save; writable: boolean }) {
  const kept = ((doc.reminders as { minutes_before?: number }[] | undefined) ?? []).map((r) => Number(r.minutes_before)).filter((n) => n > 0).sort((a, b) => b - a);
  const write = (list: number[]) => save("reminders", list.map((minutes_before) => ({ minutes_before })));
  const free = REMIND.filter((m) => !kept.includes(m));
  return (
    <div className="bk-setting bk-reminders">
      <div className="bk-setting-text">
        <span id="bk-reminders-label">{t("Reminders")}</span>
        <p className="faint">{t("Each confirmed booking reminds the guest by email. One already due when the booking is made is skipped.")}</p>
      </div>
      <div>
        <ul className="bk-chips" aria-labelledby="bk-reminders-label">
          {kept.length === 0 && <li className="muted">{t("None")}</li>}
          {kept.map((m) => (
            <li key={m} className="bk-chip">
              {isolate(before(m))}
              {writable && (
                <button type="button" aria-label={t("Remove the reminder {m}", { m: before(m) })} onClick={() => void write(kept.filter((x) => x !== m))}>
                  ×
                </button>
              )}
            </li>
          ))}
        </ul>
        {writable && free.length > 0 && (
          <select
            className="select bk-add"
            aria-label={t("Add a reminder")}
            value=""
            onChange={(e) => e.target.value && void write([...kept, Number(e.target.value)])}
          >
            <option value="">{t("Add a reminder…")}</option>
            {free.map((m) => (
              <option key={m} value={m}>
                {isolate(before(m))}
              </option>
            ))}
          </select>
        )}
      </div>
    </div>
  );
}

function Address({ doc, services }: { doc: Doc; services: { id: string; service_name: string }[] }) {
  const slug = bootstrap()?.workspace.slug ?? "";
  const server = typeof doc.public_url === "string" && doc.public_url ? doc.public_url.replace(/\/+$/, "") : (globalThis.location?.origin ?? "");
  const base = `${server}${routeBase("bookings")}book/${encodeURIComponent(slug)}`;
  const [one, setOne] = useState("");
  const url = one ? `${base}?service=${one}` : base;
  const [copied, setCopied] = useState(false);
  const copy = () =>
    navigator.clipboard
      .writeText(url)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      })
      .catch(() => toast(t("Select the address and copy it"), "error"));
  return (
    <div className="bk-address">
      <label className="field-label" htmlFor="bk-url">
        {t("The address to share")}
      </label>
      <div className="secret">
        <input id="bk-url" className="input" readOnly value={url} onFocus={(e) => e.currentTarget.select()} />
        <Button onClick={() => void copy()}>{copied ? <Check size={15} aria-hidden="true" /> : <Copy size={15} aria-hidden="true" />} {copied ? t("Copied") : t("Copy")}</Button>
        <a className="btn btn-default btn-md" href={url} target="_blank" rel="noopener noreferrer">
          <ExternalLink size={15} aria-hidden="true" /> {t("Open")}
        </a>
      </div>
      {services.length > 1 && (
        <div className="field">
          <label className="field-label" htmlFor="bk-url-one">
            {t("Send guests straight to")}
          </label>
          <select id="bk-url-one" className="select" value={one} onChange={(e) => setOne(e.target.value)}>
            <option value="">{t("Every service")}</option>
            {services.map((s) => (
              <option key={s.id} value={s.id}>
                {isolate(s.service_name)}
              </option>
            ))}
          </select>
        </div>
      )}
    </div>
  );
}

function Settings({ meta, id }: { meta: EntityMeta; id: string }) {
  const qc = useQueryClient();
  const { record, save } = useRecord(meta, id);
  // Someone else changing a setting shows here as they do.
  useLiveEntity("BookingSettings", useCallback(() => void qc.invalidateQueries({ queryKey: ["peek", "BookingSettings", id] }), [qc, id]));
  const services = useQuery({ queryKey: ["bk", "service-names"], queryFn: () => rows<{ id: string; service_name: string; active?: boolean }>("BookingService", { fields: "service_name,active", sort: "service_name asc", limit: 200 }) });
  const doc = record.data;
  if (record.error) return <ErrorState error={record.error} onRetry={() => void record.refetch()} />;
  if (!doc) return <PageLoading />;
  const writable = meta.permissions.write !== "none";
  const num = (v: unknown, fallback: number) => (typeof v === "number" ? v : fallback);
  const open = doc.public_page === true;
  const active = (services.data ?? []).filter((s) => s.active !== false);
  return (
    <div className="bk-settings">
      <section className="card" aria-labelledby="bk-online">
        <div className="card-body stack">
          <div className="bk-online">
            <h2 id="bk-online">{t("Online booking")}</h2>
            <Badge tone={open ? "green" : "neutral"}>{open ? t("Open to guests") : t("Closed")}</Badge>
          </div>
          <Switch
            id="bk-public"
            label={t("Guests can book online")}
            hint={open ? t("Anyone with the address can choose a time, without signing in.") : t("While it is off, the address shows that booking is closed. Your calendar feeds keep working.")}
            checked={open}
            disabled={!writable}
            onChange={(v) => void save("public_page", v)}
          />
          {open && <Address doc={doc} services={active} />}
          {open && active.length === 0 && <p className="muted">{t("No service can be booked yet.")} <Link to="/e/BookingService">{t("Set up a service")}</Link> {t("and the people who give it.")}</p>}
        </div>
      </section>

      <section className="card" aria-labelledby="bk-rules">
        <div className="card-body stack">
          <h2 id="bk-rules">{t("How guests book")}</h2>
          <Switch id="bk-approve" label={t("Confirm online bookings myself")} hint={t("The time is held, and the booking waits as pending until someone confirms it.")} checked={doc.approval_required === true} disabled={!writable} onChange={(v) => void save("approval_required", v)} />
          <Count id="bk-notice" label={t("Earliest a guest can book")} hint={t("Bookings and moves must start at least this long from now.")} value={num(doc.min_notice_minutes, 120)} min={0} max={525600} unit="minutes ahead" disabled={!writable} onSave={(n) => void save("min_notice_minutes", n)} />
          <Count id="bk-ahead" label={t("Furthest ahead")} hint={t("How many days ahead guests see times.")} value={num(doc.max_advance_days, 60)} min={1} max={730} unit="days" disabled={!writable} onSave={(n) => void save("max_advance_days", n)} />
          <Count id="bk-grid" label={t("A start time every")} hint={t("A service can have its own.")} value={num(doc.slot_interval_minutes, 30)} min={5} max={1440} unit="minutes" disabled={!writable} onSave={(n) => void save("slot_interval_minutes", n)} />
        </div>
      </section>

      <section className="card" aria-labelledby="bk-changes">
        <div className="card-body stack">
          <h2 id="bk-changes">{t("When guests change their minds")}</h2>
          <Count id="bk-cancel" label={t("Guests can cancel until")} hint={t("0 means until it starts.")} value={num(doc.cancellation_window_hours, 24)} min={0} max={8760} unit="hours before" disabled={!writable} onSave={(n) => void save("cancellation_window_hours", n)} />
          <Count id="bk-move" label={t("Guests can move it until")} hint={t("0 means until it starts.")} value={num(doc.reschedule_window_hours, 24)} min={0} max={8760} unit="hours before" disabled={!writable} onSave={(n) => void save("reschedule_window_hours", n)} />
          <Reminders doc={doc} save={save} writable={writable} />
        </div>
      </section>

      <section className="card" aria-labelledby="bk-more">
        <div className="card-body">
          <h2 id="bk-more">{t("Set up")}</h2>
          <ul className="bk-links">
            <li>
              <Link to="/e/BookingService">{t("Services")}</Link> <span className="faint">{t("length, price, buffers, who gives them, what to ask")}</span>
            </li>
            <li>
              <Link to="/e/BookingStaff">{t("Staff")}</Link> <span className="faint">{t("the hours they work, in their own time zone")}</span>
            </li>
            <li>
              <Link to="/e/StaffTimeOff">{t("Time off")}</Link> <span className="faint">{t("holidays and days away")}</span>
            </li>
            <li>
              <Link to="/e/BookingResource">{t("Rooms and equipment")}</Link> <span className="faint">{t("what services share")}</span>
            </li>
            <li>
              <Link to={`/e/BookingSettings/${id}`}>{t("More settings")}</Link> <span className="faint">{t("the server address, the company invoiced, the page that links point to")}</span>
            </li>
          </ul>
        </div>
      </section>
    </div>
  );
}

export function BookingPage() {
  useShellTitle(t("Booking page"));
  const meta = useEntityMeta("BookingSettings");
  const found = useQuery({ queryKey: ["bk", "settings"], queryFn: ({ signal }) => rows<{ id: string }>("BookingSettings", { fields: "public_page", limit: 1 }, signal) });
  const [starting, setStarting] = useState(false);
  const id = found.data?.[0]?.id;

  // Nothing is saved until someone first turns the page on or changes a rule.
  const start = async () => {
    setStarting(true);
    try {
      await api.post("/api/v1/BookingSettings", { public_page: false });
      await found.refetch();
    } catch (e) {
      toast(errorMessage(e), "error");
    } finally {
      setStarting(false);
    }
  };

  return (
    <div className="bk-page bk-settings-page">
      <header className="bk-head">
        <div>
          <Tabs />
          <p className="page-sub">{t("Whether guests can book online, and how.")}</p>
        </div>
      </header>
      {meta.error || found.error ? (
        <ErrorState error={meta.error ?? found.error} onRetry={() => { void meta.refetch(); void found.refetch(); }} />
      ) : !meta.data || !found.data ? (
        <PageLoading />
      ) : !id ? (
        <EmptyState title={t("The booking page is not set up yet")} icon="calendar">
          <p className="muted">{t("Start with the defaults: guests cannot book online until you open the page.")}</p>
          {meta.data.permissions.create !== "none" && (
            <Button variant="primary" loading={starting} onClick={() => void start()}>
              {t("Set up the booking page")}
            </Button>
          )}
        </EmptyState>
      ) : (
        <Settings meta={meta.data} id={id} />
      )}
    </div>
  );
}

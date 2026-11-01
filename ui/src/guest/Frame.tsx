// The public booking page's chrome: no bar and no sign-in (the platform serves it as a public page),
// just the business's name over the page, and the few things every guest page shares.

import { type ReactNode, useCallback, useState } from "react";
import { Link } from "react-router";
import { ApiError, bootstrap } from "@ferrum/client";
import { EmptyState, ErrorState, Spinner } from "@ferrum/kit";
import { browserZone, isZone, prefers12Hour } from "../zone";

export function Frame({ children, wide }: { children: ReactNode; wide?: boolean }) {
  const name = bootstrap()?.workspace.name;
  return (
    <div className="gb-page">
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <header className="gb-top">
        <span className="gb-brand">{name ?? "Bookings"}</span>
      </header>
      <main id="main" className={wide ? "gb-main gb-main-wide" : "gb-main"} tabIndex={-1}>
        {children}
      </main>
    </div>
  );
}

export function Loading({ label }: { label: string }) {
  return (
    <div className="gb-loading" role="status">
      <Spinner />
      <span className="muted">{label}</span>
    </div>
  );
}

/** A page that does not work: the address is wrong, the page is closed, or the link has run out
 *  (`home` is the booking page, where a guest can start again). */
export function Closed({ error, what, home, onRetry }: { error: unknown; what: "page" | "link"; home?: string; onRetry?: () => void }) {
  if (error instanceof ApiError && error.status === 404) {
    return what === "page" ? (
      <EmptyState title="This booking page is not open" icon="calendar">
        <p className="muted">It may be switched off for now, or the address is not quite right. Please get in touch with the business directly.</p>
      </EmptyState>
    ) : (
      <EmptyState title="This link does not work" icon="calendar">
        <p className="muted">Booking links stop working a month after the appointment, and only open the booking they were made for. Check the link in your confirmation email, or get in touch with the business.</p>
        {home && (
          <Link to={home} className="btn btn-primary btn-md">
            Book an appointment
          </Link>
        )}
      </EmptyState>
    );
  }
  return <ErrorState error={error} onRetry={onRetry} title="We could not load this page" />;
}

/** A setting the guest chose, kept in this browser (and quietly forgotten where storage is off). */
function remembered(key: string): string | null {
  try {
    return localStorage.getItem(`ferrum.bookings.${key}`);
  } catch {
    return null;
  }
}

function remember(key: string, value: string) {
  try {
    localStorage.setItem(`ferrum.bookings.${key}`, value);
  } catch {
    /* not kept */
  }
}

/** The zone times are shown in (the browser's until the guest says otherwise) and the 12 or 24 hour clock. */
export function useDisplay() {
  const [zone, setZoneState] = useState(() => {
    const kept = remembered("zone");
    return kept && isZone(kept) ? kept : browserZone();
  });
  const [hour12, setHour12State] = useState(() => {
    const kept = remembered("hour12");
    return kept === null ? prefers12Hour() : kept === "1";
  });
  const setZone = useCallback((z: string) => {
    if (!isZone(z)) return;
    setZoneState(z);
    remember("zone", z);
  }, []);
  const setHour12 = useCallback((h: boolean) => {
    setHour12State(h);
    remember("hour12", h ? "1" : "0");
  }, []);
  return { zone, setZone, hour12, setHour12 };
}

export function ClockSwitch({ hour12, onChange }: { hour12: boolean; onChange: (h: boolean) => void }) {
  return (
    <div className="segmented gb-clock" role="group" aria-label="Clock">
      <button type="button" aria-pressed={hour12} onClick={() => onChange(true)}>
        12h
      </button>
      <button type="button" aria-pressed={!hour12} onClick={() => onChange(false)}>
        24h
      </button>
    </div>
  );
}

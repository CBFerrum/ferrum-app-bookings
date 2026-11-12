// Why a screen has nothing to show, in the kit's one vocabulary: a refused read is "denied" (never shown as
// empty), no connection is "offline", anything else "failed"; every one offers the retry the caller has.

import { CalendarDays, TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";
import { ApiError, t } from "@ferrum/client";
import { Badge, StatePanel } from "@ferrum/kit";

export function Failed({ error, onRetry, title }: { error: unknown; onRetry?: () => void; title?: string }) {
  const status = error instanceof ApiError ? error.status : undefined;
  if (status === 403) return <StatePanel kind="denied" />;
  if (status === 0) return <StatePanel kind="offline" onRetry={onRetry} />;
  return <StatePanel kind="failed" title={title} error={error} onRetry={onRetry} />;
}

/** Loading with nothing yet to shape (the schedule is a grid; a booking page is a short form). */
export const Loading = ({ title }: { title?: string }) => <StatePanel kind="loading" title={title} />;

/** First use, or a day with nothing: what this is and, in `action`, the first step. */
export const Nothing = ({ title, children, action }: { title: string; children?: ReactNode; action?: ReactNode }) => (
  <StatePanel kind="empty" title={title} icon={CalendarDays} action={action}>
    {children}
  </StatePanel>
);

/** What staff read on a booking whose time a working calendar closed after it was booked: it stays, and they move it or confirm it. */
export const closedNote = () => t("A working calendar closed this time after it was booked. Move it, or confirm with the guest that it stands.");

export function ClashMark() {
  return (
    <Badge tone="amber" title={closedNote()}>
      <TriangleAlert size={12} aria-hidden="true" /> {t("Outside opening hours")}
    </Badge>
  );
}

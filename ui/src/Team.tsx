// The team's side of the interface, under the platform's bar: the records frame (the app's modules in
// the sidebar) around the schedule and the rest. It loads only when a signed-in page is opened, so the
// public booking page never carries the frame.

import { RequireAuth, setFrameMode } from "@ferrum/records/frame";

/** Bookings' packages: the sidebar shows their modules. */
setFrameMode("app", ["bookings"]);

export function Team() {
  return <RequireAuth />;
}

// The screens, drawn once in Node with the data they need already in the cache: no browser here, so
// this is what shows a screen that fails to draw (and that its words and labels are the ones people
// will read). What the pages do with a click is in the pure files' tests.

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToString } from "react-dom/server";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it } from "vitest";
import type { Bk } from "./board";
import type { Meta } from "./data";
import { BookPage } from "./guest/Book";
import type { Catalog, GuestBooking } from "./guest/booking";
import { ManagePage } from "./guest/Manage";
import { MonthGrid, Times } from "./guest/Picker";
import { NewBooking } from "./NewBooking";
import { BookingPage } from "./Page";
import { Schedule } from "./Schedule";
import { BookingSheet } from "./Sheet";
import { TimeOff } from "./TimeOff";
import { Today } from "./Today";
import { browserZone, dayBounds, dayOf, monthOf } from "./zone";

const client = () => new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } });

function draw(qc: QueryClient, url: string, page: ReactNode, path = "*"): string {
  return renderToString(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[url]}>
        <Routes>
          <Route path={path} element={page} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

const catalog: Catalog = {
  time_zone: "Asia/Kolkata",
  currency: "INR",
  services: [
    {
      id: "svc-1",
      name: "Deep tissue massage",
      description: "Sixty minutes on the knots.",
      duration_minutes: 60,
      seats: 1,
      price: "1500.00",
      location_type: "In Person",
      location: "12 High Street",
      staff: [{ id: "st-1", name: "Asha" }, { id: "st-2", name: "Ravi" }],
      questions: [
        { question: "Any allergies?", type: "Yes/No", options: [], required: true },
        { question: "Pressure", type: "Choice", options: ["Soft", "Firm"], required: false },
        { question: "Anything else", type: "Long Text", options: [], required: false },
      ],
    },
    { id: "svc-2", name: "Pilates", duration_minutes: 45, seats: 8, price: "0", location_type: "Video", staff: [{ id: "st-1", name: "Asha" }], questions: [] },
  ],
};

describe("the booking page", () => {
  it("lists what can be booked", () => {
    const qc = client();
    qc.setQueryData(["gb-catalog", "acme"], catalog);
    const html = draw(qc, "/book/acme", <BookPage />, "/book/:tenant");
    expect(html).toContain("What would you like to book?");
    expect(html).toContain("Deep tissue massage");
    expect(html).toContain("Pilates");
    expect(html).toContain("Group class");
    expect(html).toContain("service=svc-1");
  });

  it("goes straight to the times when there is one service", () => {
    const qc = client();
    qc.setQueryData(["gb-catalog", "acme"], { ...catalog, services: [catalog.services[0]] });
    const html = draw(qc, "/book/acme", <BookPage />, "/book/:tenant");
    expect(html).toContain("Choose a time");
    expect(html).toContain("Who would you like to see?");
    expect(html).toContain("Anyone free");
    expect(html).not.toContain("All services");
  });

  it("shows a closed page, not a broken one, when the page is off", () => {
    const qc = client();
    // Nothing cached and no way to ask: the loading state, not an error.
    expect(draw(qc, "/book/acme", <BookPage />, "/book/:tenant")).toContain("Loading what you can book");
  });

  it("shows the month with its free days, and a day's times", () => {
    const zone = browserZone();
    const month = monthOf(dayOf(Date.now(), zone));
    const open = new Map([
      [`${month}-15`, [{ start: "2026-11-15T04:30:00.000Z", end: "2026-11-15T05:30:00.000Z", time: "10:00", seats_left: 1, staff: [{ id: "st-1", name: "Asha" }] }]],
    ]);
    const html = renderToString(<MonthGrid month={month} first={1} open={open} selected={`${month}-15`} today={`${month}-01`} busy={false} canPrev={false} onSelect={() => {}} onMonth={() => {}} />);
    expect(html).toContain("1 time free");
    expect(html).toContain("nothing free");
    expect(html).toContain('aria-selected="true"');
    expect(html).toContain('aria-label="Previous month"');
    const times = renderToString(<Times day={`${month}-15`} slots={[...open.values()][0]} zone="Asia/Kolkata" hour12={false} at={null} showSeats={true} onPick={() => {}} />);
    expect(times).toContain("10:00");
    expect(times).toMatch(/1(<!-- -->)? left/);
  });

  it("offers the waitlist of a class that is full", () => {
    const full = { start: "2026-11-15T04:30:00.000Z", end: "2026-11-15T05:15:00.000Z", time: "10:00", seats_left: 0, full: true, staff: [{ id: "st-1", name: "Asha" }] };
    const times = renderToString(<Times day="2026-11-15" slots={[full]} zone="Asia/Kolkata" hour12={false} at={null} showSeats={true} onPick={() => {}} />);
    expect(times).toContain("Full · waitlist");
    expect(times).toContain("join the waitlist");
    const qc = client();
    qc.setQueryData(["gb-catalog", "acme"], { ...catalog, services: [catalog.services[1]] });
    const html = draw(qc, `/book/acme?at=${encodeURIComponent(full.start)}&wait=1`, <BookPage />, "/book/:tenant");
    expect(html).toContain("Join the waitlist");
    expect(html).toContain("This class is full");
  });

  it("asks for the details, with the service's questions", () => {
    const qc = client();
    qc.setQueryData(["gb-catalog", "acme"], { ...catalog, services: [catalog.services[0]] });
    const at = new Date(Date.now() + 3 * 86_400_000).toISOString();
    const html = draw(qc, `/book/acme?service=svc-1&at=${encodeURIComponent(at)}`, <BookPage />, "/book/:tenant");
    expect(html).toContain("Your details");
    expect(html).toContain("Any allergies?");
    expect(html).toContain("Pressure (optional)");
    expect(html).toContain("Confirm booking");
    expect(html).toContain("Choose another time");
    // The field only a program fills in is there, and hidden from people.
    expect(html).toContain('name="website"');
  });
});

const booking = (over: Partial<GuestBooking> = {}): GuestBooking => ({
  number: "BKG-2026-00012",
  status: "Confirmed",
  service: { id: "svc-1", name: "Deep tissue massage" },
  staff: { id: "st-1", name: "Asha" },
  start_time: "2026-11-12T04:30:00.000Z",
  end_time: "2026-11-12T05:30:00.000Z",
  time_zone: "Asia/Kolkata",
  seats: 1,
  location_type: "In Person",
  location: "12 High Street",
  guest: { name: "Jane Doe", email: "jane@example.com", phone: "98765" },
  answers: [{ question: "Any allergies?", answer: "No" }],
  amount: "1500.00",
  can_cancel: true,
  cancel_until: "2026-11-11T04:30:00.000Z",
  can_reschedule: true,
  reschedule_until: "2026-11-11T04:30:00.000Z",
  ...over,
});

describe("a guest's own booking", () => {
  const show = (b: GuestBooking) => {
    const qc = client();
    qc.setQueryData(["gb-mine", "acme", "tok"], b);
    return draw(qc, "/book/acme/manage/tok", <ManagePage tenant="acme" token="tok" />);
  };

  it("says what was booked, where and when the guest can still change it", () => {
    const html = show(booking());
    expect(html).toContain("Deep tissue massage");
    expect(html).toContain("12 High Street");
    expect(html).toContain("Asha");
    expect(html).toContain("Any allergies?");
    expect(html).toContain("Add to calendar");
    expect(html).toContain("Choose another time");
    expect(html).toContain("Cancel booking");
    expect(html).toContain("You can move it until");
    expect(html).toContain("/api/site/acme/bookings/manage/tok/booking.ics");
  });

  it("offers nothing to do after the windows have closed, and says so", () => {
    const html = show(booking({ can_cancel: false, can_reschedule: false }));
    expect(html).toContain("can no longer be changed online");
    expect(html).not.toContain("Cancel booking");
  });

  it("waits for the business to confirm a pending booking", () => {
    expect(show(booking({ status: "Pending" }))).toContain("Waiting for confirmation");
  });

  it("tells a guest on a waitlist their place", () => {
    const html = show(booking({ status: "Waitlisted", waitlist_position: 2 }));
    expect(html).toContain("number 2 in line");
    expect(html).toContain("Leave the waitlist");
  });

  it("tells a guest their unpaid booking expired", () => {
    const html = show(booking({ status: "Expired", can_cancel: false, can_reschedule: false, location: null }));
    expect(html).toContain("This booking expired");
    expect(html).toContain("deposit was not paid in time");
    expect(html).not.toContain("Add to calendar");
  });

  it("offers another time after a cancellation", () => {
    const html = show(booking({ status: "Cancelled", can_cancel: false, can_reschedule: false, cancellation_reason: "Flight changed", location: null }));
    expect(html).toContain("This booking was cancelled");
    expect(html).toContain("Flight changed");
    expect(html).toContain("Book another time");
    expect(html).not.toContain("Add to calendar");
  });
});

// ── the team's screens ──

const ZONE = "Asia/Kolkata";
const DAY = "2026-11-02";

const meta: Meta = {
  staff: [
    { id: "st-1", display_name: "Asha Rao", email: "asha@spa.test", time_zone: ZONE, active: true, hours: [{ weekday: "Monday", start_time: "09:00", end_time: "17:00" }] },
    { id: "st-2", display_name: "Ravi Iyer", email: "ravi@spa.test", time_zone: ZONE, active: true, hours: [{ weekday: "Monday", start_time: "10:00", end_time: "18:00" }] },
  ],
  services: [
    { id: "svc-1", service_name: "Massage", capacity: 1, duration_minutes: 60, active: true },
    { id: "svc-2", service_name: "Pilates", capacity: 8, duration_minutes: 45, active: true },
  ],
  resources: [{ id: "room-1", resource_name: "Studio", resource_type: "Room", active: true }],
};

const at = (hhmm: string) => new Date(Date.parse(`${DAY}T${hhmm}:00+05:30`)).toISOString();
const bk = (id: string, over: Partial<Bk>): Bk => ({
  id,
  version: 1,
  service: "svc-1",
  service_name: "Massage",
  staff: "st-1",
  staff_name: "Asha Rao",
  start_time: at("10:00"),
  end_time: at("11:00"),
  seats: 1,
  status: "Confirmed",
  session_key: id,
  guest_name: "Jane Doe",
  ...over,
});

function seeded(rows: Bk[], waiting: Bk[] = []) {
  const qc = client();
  const [from, to] = dayBounds(ZONE, DAY);
  qc.setQueryData(["me"], { tenant: "t", user: { id: "u", email: "asha@spa.test", full_name: "Asha", is_admin: false, roles: [] }, timezone: ZONE, packages: [] });
  qc.setQueryData(["meta", "Booking"], { name: "Booking", permissions: { read: "all", write: "all", create: "all", delete: "none", submit: "none", cancel: "none", export: "none", import: "none" } });
  qc.setQueryData(["bk", "meta"], meta);
  qc.setQueryData(["bk", "range", from, to], { bookings: rows, off: [{ id: "o1", staff: "st-2", from_time: at("12:00"), to_time: at("13:00"), reason: "Lunch" }] });
  qc.setQueryData(["bk", "waiting"], waiting);
  return qc;
}

describe("the schedule", () => {
  const rows = [
    bk("b1", { block_start: at("09:50"), block_end: at("11:10") }),
    bk("b2", { staff: "st-2", staff_name: "Ravi Iyer", start_time: at("10:30"), end_time: at("11:30"), guest_name: "Ben Ames", status: "Pending" }),
    bk("p1", { service: "svc-2", service_name: "Pilates", session_key: "k1", staff: "st-1", start_time: at("14:00"), end_time: at("14:45"), guest_name: "Meera", seats: 2 }),
    bk("p2", { service: "svc-2", service_name: "Pilates", session_key: "k1", staff: "st-1", start_time: at("14:00"), end_time: at("14:45"), guest_name: "Kim", seats: 1 }),
  ];

  it("draws a column for each person, with their bookings and the class as one block", () => {
    const html = draw(seeded(rows), `/?date=${DAY}`, <Schedule />);
    expect(html).toContain("Asha Rao");
    expect(html).toContain("Ravi Iyer");
    expect(html).toContain("Jane Doe");
    expect(html).toContain("Ben Ames");
    expect(html).toContain("Pilates");
    expect(html).toContain("3 of 8 seats");
    expect(html).not.toContain("Meera");
    expect(html).toContain("Time off");
    expect(html).toContain("New booking");
    expect(html).toContain("Drag a booking to move it");
  });

  it("marks a pending booking and counts what needs an answer", () => {
    const html = draw(seeded(rows, [rows[1]]), `/?date=${DAY}`, <Schedule />);
    expect(html).toContain('data-status="Pending"');
    expect(html).toContain("1 to confirm");
    expect(html).toContain("Confirm");
  });

  it("draws a week by day, for one person", () => {
    const qc = seeded(rows);
    const html = draw(qc, `/?view=week&date=${DAY}&staff=st-1`, <Schedule />);
    // The week's range is not cached: the board shows the people's week while it loads.
    expect(html).toContain("Mon");
    expect(html).toContain("Sun");
  });

  it("says when nothing is booked", () => {
    expect(draw(seeded([]), `/?date=${DAY}`, <Schedule />)).toContain("Nothing is booked");
  });
});

describe("today's list", () => {
  const minutes = (n: number) => new Date(Date.now() + n * 60_000).toISOString();

  it("shows a loading state until the day arrives", () => {
    expect(draw(seeded([]), "/today", <Today />)).toContain("Loading today");
  });

  it("puts the day in the order it happens", () => {
    const qc = seeded([]);
    const [from, to] = dayBounds(ZONE, dayOf(Date.now(), ZONE));
    const rows = [
      bk("old", { start_time: minutes(-180), end_time: minutes(-120), guest_name: "Over Done" }),
      bk("now", { start_time: minutes(-30), end_time: minutes(30), guest_name: "Under Way" }),
      bk("soon", { start_time: minutes(120), end_time: minutes(180), guest_name: "Coming Soon", status: "Pending" }),
      bk("fin", { start_time: minutes(-400), end_time: minutes(-340), guest_name: "Finished One", status: "Completed" }),
    ];
    qc.setQueryData(["bk", "day", from, to], { bookings: rows, off: [] });
    qc.setQueryData(["bk", "waiting"], [rows[2]]);
    const html = draw(qc, "/today", <Today />);
    for (const word of ["Needs your answer", "Did they come?", "Under way", "Coming up", "Done", "Over Done", "Under Way", "Coming Soon", "Finished One"]) expect(html).toContain(word);
    expect(html.indexOf("Did they come?")).toBeLessThan(html.indexOf("Under way"));
    expect(html.indexOf("Under way")).toBeLessThan(html.indexOf("Coming up"));
    expect(html).toContain("Came");
    expect(html).toContain("No show");
    expect(html).toContain("Mine");
  });

  it("says so when nothing is booked", () => {
    const qc = seeded([]);
    const [from, to] = dayBounds(ZONE, dayOf(Date.now(), ZONE));
    qc.setQueryData(["bk", "day", from, to], { bookings: [], off: [] });
    expect(draw(qc, "/today", <Today />)).toContain("Nothing booked");
  });
});

describe("the booking page's settings", () => {
  it("shows whether guests can book, the address to share and the rules", () => {
    const qc = client();
    qc.setQueryData(["meta", "BookingSettings"], {
      name: "BookingSettings",
      permissions: { read: "all", write: "all", create: "all", delete: "none", submit: "none", cancel: "none", export: "none", import: "none" },
      fields: [],
      form: [],
      children: {},
    });
    qc.setQueryData(["bk", "settings"], [{ id: "set-1" }]);
    qc.setQueryData(["peek", "BookingSettings", "set-1"], { id: "set-1", version: 3, public_page: true, approval_required: false, min_notice_minutes: 120, max_advance_days: 60, slot_interval_minutes: 30, cancellation_window_hours: 24, reschedule_window_hours: 12, reminders: [{ minutes_before: 1440 }, { minutes_before: 60 }] });
    qc.setQueryData(["bk", "service-names"], [{ id: "svc-1", service_name: "Massage", active: true }, { id: "svc-2", service_name: "Pilates", active: true }]);
    const html = draw(qc, "/page", <BookingPage />);
    for (const word of ["Open to guests", "The address to share", "Guests can book online", "Confirm online bookings myself", "Earliest a guest can book", "1 day before", "1 hour before", "Send guests straight to", "More settings"]) expect(html).toContain(word);
    expect(html).toContain("book/");
  });

  it("offers to set the page up when nothing was saved yet", () => {
    const qc = client();
    qc.setQueryData(["meta", "BookingSettings"], { name: "BookingSettings", permissions: { read: "all", write: "all", create: "all", delete: "none", submit: "none", cancel: "none", export: "none", import: "none" } });
    qc.setQueryData(["bk", "settings"], []);
    expect(draw(qc, "/page", <BookingPage />)).toContain("Set up the booking page");
  });
});

describe("a booking's sheet and the dialogs", () => {
  const sheet = (b: Partial<Bk> & Record<string, unknown>, canWrite = true) => {
    const qc = client();
    qc.setQueryData(["bk", "doc", "b1"], { ...bk("b1", {}), guest_email: "jane@example.test", guest_phone: "98765", answers: [{ question: "Any allergies?", answer: "No" }], notes: "Left shoulder", amount: "1500.00", number: "BKG-2026-00001", ...b });
    return draw(qc, "/", <BookingSheet id="b1" zone={ZONE} hour12={false} canWrite={canWrite} onClose={() => {}} />);
  };

  it("shows when, who, what they said and what it costs, with the buttons its status allows", () => {
    const html = sheet({ start_time: new Date(Date.now() + 86_400_000).toISOString(), end_time: new Date(Date.now() + 90_000_000).toISOString() });
    for (const word of ["Jane Doe", "jane@example.test", "98765", "Any allergies?", "Left shoulder", "1,500", "BKG-2026-00001", "Move", "Cancel", "More", "History"]) expect(html).toContain(word);
    expect(html).not.toContain("Confirm</");
    expect(html).not.toContain("Completed</button>");
  });

  it("offers Confirm and Decline for a pending booking, and nothing to someone who cannot write", () => {
    const pending = sheet({ status: "Pending", start_time: new Date(Date.now() + 86_400_000).toISOString() });
    expect(pending).toContain("Confirm");
    expect(pending).toContain("Decline");
    expect(sheet({ status: "Pending" }, false)).not.toContain("Decline");
  });

  it("warns on a booking a working calendar closed, and says an expired one let go of its time", () => {
    const soon = new Date(Date.now() + 86_400_000).toISOString();
    const clash = sheet({ start_time: soon, closed_clash: true });
    expect(clash).toContain("closed this time after it was booked");
    expect(sheet({ start_time: soon })).not.toContain("closed this time");
    const expired = sheet({ status: "Expired", start_time: soon });
    expect(expired).toContain("Expired");
    expect(expired).toContain("deposit was not paid in time");
  });

  it("asks who and when for a new booking, with the person and the start filled in", () => {
    const html = renderToString(
      <QueryClientProvider client={client()}>
        <MemoryRouter>
          <NewBooking meta={meta} zone={ZONE} hour12={false} from={{ staff: "st-2", start: Date.parse(`${DAY}T10:15:00+05:30`) }} onClose={() => {}} onCreated={() => {}} />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    for (const word of ["New booking", "Choose a service", "Anyone free", "Ravi Iyer", "Guest&#x27;s name", "All fields"]) expect(html).toContain(word);
    expect(html).toContain('value="2026-11-02"');
    expect(html).toContain('value="10:15"');
  });

  it("blocks out time for a person", () => {
    const html = renderToString(<TimeOff meta={meta} zone={ZONE} day={DAY} onClose={() => {}} onSaved={() => {}} />);
    for (const word of ["Block out time", "Who is away", "Whole days", "Asha Rao", "Reason"]) expect(html).toContain(word);
  });
});

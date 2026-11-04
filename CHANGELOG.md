# Changelog

## 0.3.1

Rebuilt with Ferrum-Frontend 80ff179: a field's label names its control for assistive technology, and a record whose title is a link shows that record's title, not its id.

## 0.3.0, and Booking Deposits 0.1.0

Deposits and refunds (FERRUM-BOOKINGS-003), in a connector app with Commerce, Booking Deposits,
offered where both are installed.

- A service can take a deposit (a share of its price) when a guest books it, refunded whole when the
  booking is cancelled at least so many hours ahead, and kept in part (a share of it) later.
- A guest's booking of such a service waits as Pending: it is invoiced at once, and the deposit is
  asked for through the store's payment method named in Booking Settings. The guest's booking view
  shows it and where to pay it. Paying confirms the booking; the rest is paid on the day.
- Cancelling it settles the deposit by the terms: an unpaid one stops being asked for and its invoice
  is cancelled; a paid one is kept in part, the invoice credited for the rest, and what was paid
  beyond what is kept goes back through the gateway once a person approves the refund (Commerce 0.7.0).

Not yet: a deposit for a booking staff make (only guests' bookings ask for one), and asking for the
rest online before the day.

## 0.2.0

- A booking page guests can use. At `/bookings/book/{workspace}`, with no sign-in: choose a service,
  a person when more than one gives it, a day on a month calendar (days with times are marked, and
  the first is chosen for you) and a time, shown in the guest's own time zone with a 12 or 24 hour
  clock. Then their details and the service's questions, with the server's complaints beside the
  field they are about. A time taken while they typed disables Confirm with a message. A full class
  offers its waitlist. Phones get the same steps, with the button to book where a thumb is.
- A guest's own booking page, which the links in their emails now open: what was booked and where,
  add to a calendar (a file, Google, Outlook), choose another time from what is really free, or
  cancel with a reason, for as long as the business allows.
- The schedule, the team's home: a day by person (or by room) or a week by day, bookings sized to
  how long they last with their buffers shaded, working hours lit, time off hatched, a line at the
  current time, and a class as one block with its seats. Press an empty time to book it with the
  person and start filled in. Drag a booking to move it, or press M on it and use the arrow keys:
  it shows at once and goes back, with the server's reason, if refused. Bookings that wait for an
  answer sit in a strip above, to confirm or decline.
- A booking opens in a sheet: when, who, where, what they answered, what it costs and the history,
  with the buttons its status allows. Moving it shows real free times (its own time counts as free
  to it), cancelling asks for a reason and says whether the guest is told, and a booking can be
  repeated or invoiced from it.
- Today: what needs an answer, what is under way, what comes next, and what is over and still to be
  closed (did they come?), for the person's own bookings or everyone's.
- Booking page: open or close online booking, copy the address (or a service's own), and change the
  rules people change most: confirming by hand, notice, how far ahead, when guests can still cancel
  or move, and reminders.
- Guest links in emails and calendar files lead to that page, unless the business has set a page of
  its own. Asked with `waitlist=1`, the public slot list also names a full class, marked full, so a
  page can offer its waitlist (the list is as it was without it); Find Free Slots takes the booking
  being moved, so its own time is not offered as taken.

- Repeating bookings: the Repeat action makes a booking the first of a series, weekly, every 2 or 4
  weeks or monthly, at the same local time for the staff member across a change of the clocks. Each
  occurrence is an ordinary booking, placed and checked as any other. A series is booked whole, or
  names the date that is taken and books nothing, unless the caller asks to skip what does not
  fit. Cancel This and Later ends one from any point on.

## 0.1.1

- Requires the ERP app (Ferrum docs/plan/apps-ui.md §13): installing it in a workspace without ERP
  installs ERP whole, with its interface, rather than a few of its packages.
- Declares where it keeps people's personal data, so Administration → Personal data exports and
  erases it for a person (by email).
- Guests get their emails without the Mail app too: when it is not installed, notices and
  reminders go through the server's own mailer, with an add-to-calendar link.
- The public write throttle (30 booking changes per address in ten minutes) is counted in the
  database, so it holds however many replicas serve the booking page.

## 0.1.0

- First release. Appointment scheduling on Ferrum: services as one-to-one appointments or group
  classes (duration, buffers, price and invoicing item, seats, locations, intake questions), staff
  with their own time zones, split working hours and time off, and capacity-limited rooms or
  equipment. A pure availability engine (DST-correct, slot grid, notice and advance windows,
  buffers, seats, resources, fair "any staff" choice) decides both the slots offered and the
  bookings taken, under ordered row locks and a database exclusion constraint, so double bookings
  cannot happen even under concurrent requests. Full classes keep waitlists that fill themselves
  when seats free up. A public booking page API (services, slots in the guest's zone, booking with
  validation, honeypot and throttling, token-based self-service to view, reschedule and cancel
  within windows, `.ics` downloads) sits next to document actions for staff and AI agents
  (confirm, reschedule, cancel, complete, no-show, invoice once at the booked price, guest links,
  free-slot search, signed calendar feeds). Maintenance sends every notice and reminder exactly
  once — email with calendar invitations through the Mail app when installed, in-app for staff —
  and three reports cover volume, attendance and staff utilization.

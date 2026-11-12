# Changelog

## 0.7.1

- **Dated exceptions of a platform calendar are honoured.** Availability, the **Outside Opening Hours** mark and the Staff Utilization report now take open time from the platform calendar's own functions, so its hours, weekends, holidays and dated exceptions are one source: a Saturday opened by an exception is offered, a Wednesday closed from 09:00 is not, a closed afternoon or an opened holiday works, also on a day the clocks change. A missing or unusable calendar still counts as closed, and staff without a calendar keep their weekly hours.
- **Closed and Extra Open Dates in Booking Settings are superseded.** The first upkeep after the upgrade moves each row into the exceptions of the calendar it names (the business calendar when it names none; when the workspace had none, a calendar `bookings_business`, open all day in the workspace's zone, is made and named), never over an exception the calendar already has for that date, one per date (the first row wins). The rows stay readable; rows entered afterwards are not read. One difference: an extra opening on a weekday now replaces that day's hours instead of adding to them.

## 0.7.0

- **A video meeting for each booking** (BK-48). A Video service that states no link of its own gives each booking its own meeting in the Mail, Chat and Calendar app (needs Comms 0.15.0 and a meeting server) once the booking is confirmed, hosted by the booked staff member. The guest's own join link, which needs no account and admits only to that meeting, becomes the booking's Where: the booking page, the confirmation, the reminders and the calendar entry all carry it. Rescheduling a booking moves its meeting (the link stays the same); cancelling or expiring it ends the meeting and the link stops working. Without Comms, without a meeting server, without the workspace's public address in Comms settings, or when the service states its own link, bookings are exactly as before; a meeting that cannot be made never stops a booking. A class's guests each get their own meeting for now.
## 0.6.2

The interface knows what 0.6.0 added. A booking whose time a working calendar closed shows a warning mark, **Outside opening hours**, with what to do (move it, or confirm with the guest): on the schedule's block, on Today, on a pending booking waiting for an answer and at the top of its sheet. **Expired** is muted like Cancelled, holds no block on the schedule, and a guest opening it is told it expired and why. The new fields' and options' names (working calendars, per-service rules, the unpaid-deposit hold, Expired, Restored, Refunded) are in Arabic and Hindi too. Rebuilt with Ferrum-Frontend 0a47157.

## 0.6.1

Rebuilt with Ferrum-Frontend 45dad7f, the design system as tuned after the landed build was reviewed. The left menu's icons are chosen so that none repeats on more than three rows and each fits its row; their drawings load beside the first page instead of inside it, so the page starts lighter; a row that scrolls sideways fades at the edge where there is more, in every browser and with reduced motion; a printed record leaves out empty fields and keeps its totals with the table; menus and popovers sit inside the page's landmarks for assistive technology; a selection bar folds what does not fit into More on a narrow screen. Nothing else changes.

## 0.6.0

- **Working calendars.** A staff member, a resource and Booking Settings may name a working calendar from Admin, Calendars. A staff member is then bookable in that calendar's open time (inside the business calendar's, when one is set), so a holiday entered once closes every service; a resource only in its calendar's. Booking Settings keeps **Closed and Extra Open Dates** per calendar (empty: the whole business), for a one-off closure or an extra open Saturday. Staff without a calendar keep their weekly hours as before. A booking whose time a calendar later closed stays and is marked **Outside Opening Hours**.
- **Rules per service.** A service may set its own minimum notice, how far ahead, cancellation and reschedule windows, manual confirmation and reminders; empty ones follow Booking Settings.
- **Unpaid deposits** (Booking Deposits 0.2.0). A booking waiting for its deposit holds its time for **Unpaid Deposit Holds Its Time** minutes (on the service or the settings), or else until its deposit request expires. Then it is **Expired**, its time released and the guest told. Paid late, it is restored when its time is still free, and refunded in full when it was taken.
- **Guests are customers at once.** A booking links the guest's ERP Customer, found by email (or phone digits) or made, when it is booked rather than when it is invoiced.

## 0.5.1

The interface follows the upgraded design system, rebuilt with Ferrum-Frontend 0a47157. Text uses the kit's type roles (nothing under 12px) and controls its height tokens; loading shows skeletons shaped like what is coming; empty, no matches, denied, offline and failed are told apart; status colours come from stored state ids. The interface check now runs the UI lint against an empty baseline. The schedule's hour labels no longer wrap.

## 0.5.0

A booking whose staff member's outside calendar becomes busy at its time after it was booked is marked **Clashes With the Staff Calendar** (FEO-032):
- The staff member is told in-app.
- The guest's page says their host is no longer free then, and offers another time where the booking can still be moved.
- Moved to a free time, the mark goes; the calendar free again, it goes too.
- Keep Despite the Clash leaves the booking at its time, and it is not marked again there; moved, it is checked again.

What stays as it was:
- A guest repeats a booking from the page only for a free service.
- A series a person repeats is the business's bookings at the guest's request: invoiced, without a deposit per date.
- A no-show keeps its deposit, and only a cancellation settles one.

Rebuilt with Ferrum-Frontend 9f5a466, with the new fields and words in Arabic and Hindi.

## 0.4.18

The reports it defines in code (their titles and parameters) have names in Arabic and Hindi too, with its models' and manifest's (250 each in ui/src/locales/names). Translated by the model, to be reviewed by native speakers.

## 0.4.17

Rebuilt with Ferrum-Frontend c59c9a2: the names of its models, fields and manifest in Arabic and Hindi (249 each in ui/src/locales/names, which the platform serves to every page of the workspace, so a record's labels, the launcher and the bar read in the person's language), and the sentences its pages still showed in English (split around a link or a figure, built in code from a template or word by word) whole and translated (370 words each in ui/src/locales). Translated by the model, to be reviewed by native speakers.

## 0.4.16

Rebuilt with Ferrum-Frontend 155518c: the sentences its pages built in code (counts, dates, what a toast or a screen reader says) go through t() too, as whole sentences where code chose a word, and carry built-in Arabic and Hindi (339 words each in ui/src/locales, translated by the model and to be reviewed by native speakers). The later dates of a guest's series are recorded as booked from the page, as the first is.

## 0.4.15

A guest books a series from the booking page: every week, 2 weeks, 4 weeks or month, as many times in all as the service lets them (BookingService.guest_repeat_max, 0 by default), each at the same local time. It is booked whole or not at all, and a date that is taken is named; the confirmation lists the later dates. Only free services repeat for guests: a paid one is booked one at a time, its deposit being the first booking's. The guest page's new words are in Arabic and Hindi.

## 0.4.14

The assistant knows what its records mean in its pages (contributes.shell.assistant): their states, what counts and where figures come from, with questions it suggests asking there.

## 0.4.13

Rebuilt with Ferrum-Frontend 5b22e7b: its own words carry built-in Arabic and Hindi (305 words each, ui/src/locales, translated by the model and to be reviewed by native speakers), fetched for the page's language only. The labels it keeps in objects, its dialogs, its page titles and the words it keeps in variables go through t() as well, and the names and sizes it shows as they are keep their order right to left. Its pages are drawn once their words are in, and its entry carries only the parts of the kit it uses.

## 0.4.12

Rebuilt with Ferrum-Frontend 6537d65: an action's link, code or token shown once is kept in a dialog to copy, and on a page that reads right to left, English left untranslated keeps its own order. Its own words go through the kit's t() (272 places), so a workspace's own words for its language reach them, and Admin, Interface words lists them under it (dist/words.json).

## 0.4.11

Rebuilt with Ferrum-Frontend d5d98f2: a workspace's own words for its interface (Admin, Interface words) reach its pages as they load; an attachment field that holds a picture shows it; and a value another record already has is said so, where the form said the record had been changed by someone else. Its own screens' margins, paddings, borders, alignment and insets go by the line's start and end, so they mirror right to left as the kit's do; what it places by coordinates (boards, timelines, charts, handles) stays as it was.

## 0.4.10

Rebuilt with Ferrum-Frontend 63280d3: right to left, its pages mirror as they read (icons that point along the line, margins and alignment by the line's start and end, arrow keys along it), and the records pages' words are in Arabic and Hindi.

## 0.4.9

Rebuilt with Ferrum-Frontend a64416a: a new record's box whose default is 1 shows ticked, as it is saved.

## 0.4.8

Rebuilt with Ferrum-Frontend 922fcfb: an action's dialog says every refusal, the errors on what the action made (another record's rows) among them, and an action with parameters and a confirmation asks once.

## 0.4.7

Rebuilt with Ferrum-Frontend a76295e: its pages show times on a person's own clock when they chose one on their Account page, while today and due dates stay on the workspace's day.

## 0.4.6

A booking's history is on the workspace's clock, as the team's board is; guests keep their own. Rebuilt with Ferrum-Frontend 24c3951, so its records' pages show times on the workspace's clock too, a draft numbered on submit is headed as a draft, and a record of an app that is not installed says so.

## 0.4.5

Rebuilt with Ferrum-Frontend bb60115: a record page reads an empty number or check as the server does, so an action whose condition needs one shows, and a workspace's settings open as their one record.

## 0.4.4

A booking's record page offers what it can do: Repeat asks how often, how many bookings in all and whether to skip times that are taken, and Cancel Booking and Cancel This and Later ask for the reason the guests see. They read those and never asked, so from a record page Repeat always failed and cancelling kept no reason. Reschedule and Find Free Slots need a time and show free times, which the schedule's booking sheet does: a record page no longer offers them as buttons.

## 0.4.3

Rebuilt with Ferrum-Frontend 00dea07: in a workflow state that names what may change, the other fields show read-only, and the fields the state needs show required.

## 0.4.2

Personal-data requests find booking records by the person's phone number as well as their email, and list the booking records holding their name apart, for an administrator to judge (never erased for the name alone) (Ferrum's FERRUM-PRIVACY-002).

## 0.4.1

Rebuilt with Ferrum-Frontend daeadda: after an action runs, a record page shows what the action said it did rather than only that it is done.

## 0.4.0

Outside calendars (FERRUM-BOOKINGS-004): what a staff member's own calendar elsewhere says now
keeps guests from those times, as a booking does.

- A staff record takes the address of a calendar of theirs in iCalendar format (Google Calendar's
  secret address in iCal format, Outlook's published ICS link, an iCloud or Nextcloud subscription
  link, Ferrum Comms' own feed). It is a secret: it opens their calendar, so it is never shown again
  or repeated in a message.
- Every quarter of an hour the upkeep reads it through the platform's vetted client (https, never
  a private address, 15 seconds, 5 MB) and keeps only when they are busy, never what an event is.
  A time it covers, with the service's buffers, is not offered ("busy in their own calendar").
- Events marked free (`TRANSP:TRANSPARENT`, Outlook's free) or cancelled are not busy. Recurring
  events are expanded with their exceptions, and a moved occurrence counts where it went, by the
  icalendar crate (rrule underneath) rather than by hand; all-day events and floating times are
  read in the staff member's zone, Outlook's Windows zone names are mapped, and a zone nobody knows
  is taken as the staff member's own.
- A reading that fails says why on the staff record (Outside Calendar Problem), and what was read
  before still counts; an address taken off the record takes its busy times with it.

Not yet: a booking already made is not flagged when an outside calendar later says the staff
member is busy then; nothing is written to the outside calendar beyond the feeds and invitations
Bookings already sends.

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

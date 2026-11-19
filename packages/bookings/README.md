# bookings — appointments and classes

Package `bookings` (depends on `core`, `selling`, `accounting`): services, staff with their own
time zones and working hours, time off, resources, an availability engine, bookings with
waitlists, a public booking page API, notifications and reminders, invoicing and reports.

Everything is records and document actions, so the generic API (`/api/v1/...`) and MCP operate
it fully; the endpoints under `/api/site/{tenant}/bookings` exist for guests (and calendar apps).
Times are RFC 3339 UTC with milliseconds (`2026-11-02T04:30:00.000Z`); decimals are strings.
Errors use the usual API shape `{"error", "message", "fields"?}`: `not_found` (404),
`forbidden` (403), `validation_failed` / `invalid` (422), `conflict` (409),
`too_many_requests` (429).

## Roles

| Role | Can |
|---|---|
| Bookings Manager | everything below on every record: services, staff, working hours, time off, resources, settings, all bookings (confirm, reschedule, cancel, complete, no-show, invoice, guest links); customers; creates (draft) Sales Invoices |
| Bookings User | a staff member: reads services, staff, resources and settings; reads and works **their own bookings** (assigned to them — `staff_email` — or created by them) and creates bookings; enters, edits and deletes **their own time off**; gets their own calendar feed link |

Administrators can do everything. Staff without a Bookings role see nothing of the app.

## Setup

1. **Booking Settings** (`BookingSettings`, one per workspace; defaults apply until saved):

   | Field | Default | Meaning |
   |---|---|---|
   | `company` | — | Company bookings are invoiced from (a booking may name another) |
   | `public_page` | off | Opens the public endpoints below (except staff feeds, which always work). Off: they answer 404 |
   | `approval_required` | off | Public bookings start `Pending` (holding their time) until staff confirm them |
   | `min_notice_minutes` | 120 | Public bookings and reschedules start at least this far from now |
   | `max_advance_days` | 60 | ... and at most this far ahead |
   | `slot_interval_minutes` | 30 | Start times are offered every this many minutes from the start of each working interval |
   | `cancellation_window_hours` / `reschedule_window_hours` | 24 / 24 | Guests may cancel / move up to this many hours before the start (0 = until the start). Waitlist places can always be cancelled |
   | `reminders` | none | Rows of `minutes_before` (5–43200, unique), e.g. 1440 and 60 |
   | `public_url` | the server's | This server's internet address, for absolute links in emails and feed URLs. Empty: a guest's links (in emails and `guest_link`) are the platform's for people outside, through the operator's link host when there is one (`FERRUM_LINK_HOST`), so they keep working when the workspace's address changes: the booking page through `/go/{workspace}/s` (only the workspace's main host serves it), the endpoints through `/go/{workspace}/p`; a staff feed starts where services call back (`FERRUM_HOOKS_HOST`, else the server's public address); with no address at all, they are relative, as before |
   | `manage_page_url` | — | Your own booking page; guest links become `<url>?token=<token>` (else they open the booking page this app serves, `/bookings/book/{workspace}/manage/{token}`) |
   | `signing_secret` | made on first use | **Secret field** (needs `FERRUM_SECRET_KEY`): the HMAC key of staff feed links. Write-only; replacing it retires every feed link |

2. **Booking Staff** (`BookingStaff`): `display_name` (shown to guests), `email` (unique; the
   Ferrum user who is this staff member — owner of their bookings, receiver of in-app notes,
   organizer of invitations), `time_zone` (IANA, default the workspace's), `active`, `bio`,
   `working_hours` rows `{weekday: Monday…Sunday, start_time: "HH:MM", end_time: "HH:MM"}` in the
   staff member's zone. `24:00` is midnight; several rows a day leave breaks; rows of one day may
   not overlap; a night shift is one row per day (touching intervals merge, so an appointment may
   run past midnight).
3. **Staff Time Off** (`StaffTimeOff`): `staff`, `from_time`, `to_time`, `reason`. No new
   appointment may overlap it; existing ones stay (`conflicting_bookings` counts the active ones
   inside it when saved) until rescheduled or cancelled. Bookings Users enter time off for
   themselves only.
4. **Booking Resources** (`BookingResource`): `resource_name`, `resource_type` (Room, Equipment,
   Vehicle, Other), `capacity` — how many bookings or class sessions it hosts at the same
   instant — and `active`.
5. **Booking Services** (`BookingService`):

   | Field | Meaning |
   |---|---|
   | `service_name`, `description`, `active` | Only active services can be booked |
   | `duration_minutes` | 5–1440 |
   | `buffer_before_minutes`, `buffer_after_minutes` | Kept free around each booking (each booking keeps its own buffers) |
   | `slot_interval_minutes` | Overrides the settings' grid |
   | `capacity` (Seats) | 1 = one-to-one; more = a class with that many seats per session |
   | `price`, `item` | Per seat in the company currency; an `item` is required when the price is above 0 |
   | `location_type`, `location_details` | In Person / Video / Phone; video links and phone numbers are shown to booked guests only |
   | `sessions_only` (Only Scheduled Sessions) | A class is offered only at its scheduled Class Sessions (step 6). Off (default): any free grid time opens a session, and scheduled sessions are offered as well |
   | `staff` | Rows `{staff, priority, weight}` — who provides it (at least one); see "Any staff" below |
   | `resources` | Rows `{resource}` — each booking needs one of them with a free unit (empty = none) |
   | `questions` | Intake questions `{question, answer_type: Text / Long Text / Number / Yes/No / Choice, options (one per line, for Choice), required}` |

6. **Class Sessions** (`BookingSession`, numbered `SES-{YYYY}-{seq:5}`; Bookings Manager creates,
   changes and deletes, Bookings User reads): one dated session of a group class scheduled before
   anyone books it, with `service` (a class: more than one seat), `staff` (who gives the service,
   active), `start_time`, optional `capacity` (its own seats, 1–1000; empty = the service's) and
   optional `resource` (one of the service's rooms, active; empty = a free room is taken when the
   first seat is booked). Scheduling refuses a teacher who is not free then (outside hours, time
   off: a field error on `start_time`; another booking or session in the way, buffers included:
   409), and a room with no free unit (409). A scheduled session holds its teacher's time and its
   room from then on, with nobody booked: no other booking or session can take them. Its key is
   `s/{id}`, shared by every seat booked into it. Afterwards only `capacity` changes by an edit,
   never below the seats taken; the time, teacher, room and status change only through the
   session's actions (below). It is deleted only while nothing is booked or waitlisted in it. Seats booked
   before at the same time (an ad hoc session) are not taken over: scheduling there is refused.

7. **Booking Locations** (`BookingLocation`; Bookings Manager creates and changes, Bookings User
   reads): `location_name`, `active`, `address` (guests see it for an in-person service that states no
   address of its own), `time_zone` (IANA; where the booking page starts for it), `calendar` (Opening
   Calendar: nobody is booked there outside its open time; empty = the business calendar of Booking
   Settings). A staff member (`location`) and a room (`location`) belong to one location or, empty,
   to every location; a person who works at two places is "everywhere" (hours per location are not
   built). A service lists where it is offered (`locations` rows; empty = every location). A booking
   and a class session carry `booking_location`, set once when made and kept by Reschedule, Move,
   a waitlist promotion and a paid deposit's restore; an edit cannot change it (cancel and book
   again). A location in use cannot be deleted: make it inactive, and its bookings stay.

   **Which location**: with no active location, none (as before locations existed). Otherwise the
   one asked for (it must be active and one the service is offered at, else a field error on
   `location`); else the named staff member's when the service is offered there; else the only
   location the service is offered at; else "choose a location". Only that location's staff and
   rooms (and those of every location) are candidates; a named staff member who works elsewhere is
   refused on `staff`. `find_slots` and `/slots` take `location` the same way.

## How availability works

* The appointment `[start, start + duration)` lies inside the staff member's working time (their
  zone; DST handled: a skipped local time means the first instant after the gap, a repeated one
  its first occurrence) and outside their time off.
* Each booking blocks `[start − buffer before, end + buffer after)`. Blocks of different
  sessions of one staff member never overlap — the engine checks it, and the database enforces
  it with an exclusion constraint (`bookings_no_double_booking`). Buffers may reach outside
  working hours.
* Seats of a class session (same service, staff member and start) share one block; a session is
  full at `capacity` seats — a scheduled session's own when it has them. Existing and scheduled
  sessions are offered at their own start even off the grid; a class with Only Scheduled Sessions
  offers nothing else ("no session is scheduled then").
* A scheduled Class Session is busy time for its teacher for every booking that is not one of its
  seats, and counts on its room while empty.
* A resource hosts at most `capacity` sessions at any instant; joining a class uses the
  session's resource. The least loaded eligible resource is taken.
* Guests (public endpoints) book only offered starts — on the grid from the start of each
  working interval — inside the notice and advance windows. Staff (internal bookings, the
  generic API) may book any minute inside working hours, past or future.
* "Any staff" (`staff` empty): join an existing class session first; otherwise, of the staff free
  then, the highest `priority` on the service's staff row (0 to 100, default 0); among equal
  priorities, the fewest sessions that local day per unit of `weight` (1 to 100, default 1), then
  the fewest booked minutes per unit of weight, then the heavier weight, then a stable order.
  Compared in whole numbers, so two requests for one time agree. Rows saved before 0.13.0 read as
  priority 0, weight 1: today's order.
* Concurrency: every booking, reschedule and waitlist promotion locks its candidate staff rows,
  then their resources, each in id order, before reading the calendar and deciding; seats and
  resource units are counted under those locks.

## Booking (`Booking`)

Numbered `BKG-{YYYY}-{seq:5}`. Create through `POST /api/v1/Booking` with `service`,
`start_time`, optional `staff` (empty = any), `seats` (default 1), `customer` or `guest_name`
(plus `guest_email`, `guest_phone`, `guest_time_zone` — IANA, default the workspace's),
`answers` rows `{question, answer}`, `notes`, `company`, `join_waitlist`. The server fills
`staff`, `staff_name`, `resource`, `end_time`, `status`, `service_name`, `location_type`,
`location`, `item`, `rate` (the service price now), `amount` (rate × seats), `company` (from the
settings) and guest details from the customer. Without a `customer` the booking gets one when it is made: a public booking's is made from what the guest typed, never found by that address; a staff booking's is found by email or phone digits, or made. Staff bookings are `Confirmed` at once; a full
class with `join_waitlist` gives `Waitlisted`; an unavailable time is `409` (taken: busy, full,
no resource) or `422` (outside hours, too soon or far ahead, off the grid) with the reason.

Statuses: `Pending`, `Confirmed` (both hold time), `Waitlisted`, `Cancelled`, `Completed`,
`No Show`. Times, staff, service and seats change only through `reschedule`; statuses only
through the actions. Guest details, answers and notes can be edited. Active bookings cannot be
deleted (cancel them). `source` is `Internal` or `Public`.

### Actions — `POST /api/v1/Booking/{id}/actions/{action}`

| Action | When | Params → result |
|---|---|---|
| `confirm` | Pending | → the booking, `Confirmed` |
| `reschedule` | Pending, Confirmed | `{start_time, staff?: id \| "any", seats?}` (staff absent = keep) → the booking; re-checked like a new booking (staff rules); freed class seats go to the waitlist |
| `cancel_booking` | Pending, Confirmed, Waitlisted | `{reason?}` → the booking, `Cancelled`, `cancelled_by: Guest` (a cancellation the guest asked for, here or on their page): a paid deposit is settled by the service's terms; a freed class seat goes to the waitlist |
| `cancel_by_business` (We Cancel It) | Pending, Confirmed, Waitlisted | `{reason?}` → the booking, `Cancelled`, `cancelled_by: Business`: the business lets the time go, so a paid deposit comes back in full whatever the service's terms; otherwise as `cancel_booking` |
| `complete` | Confirmed, once started | → `Completed` |
| `mark_no_show` | Pending, Confirmed, once started | → `No Show` |
| `make_sales_invoice` | Confirmed, Completed, No Show | → `{entity: "SalesInvoice", id, number}`: a draft invoice of the booking's company in its currency, one line `item × seats` at the booked `rate`, no discount (the company's default sales taxes apply). A guest without a customer becomes one: a booking from the public page a customer of its own, made from what the guest typed (never one found by that address); a staff booking's guest as ERP finds a person's (`portal::customer_for`: an enabled customer with the guest's email, else phone digits, that no web shop's guest checkout made, else a new individual). Refused while a draft or submitted invoice exists; a deleted draft (the link clears) or a cancelled invoice allows a new one. Needs Sales Invoice create permission |
| `guest_link` | not Cancelled | → `{token, url, api, expires_at, note}`: a fresh self-service link to hand to the guest (the fallback without the Mail app) |

### Class Session actions — `POST /api/v1/BookingSession/{id}/actions/{action}`

Bookings Manager only (write on Class Session). Each is the operation `bookings.booking_session.{action}`
(`transactional_action`; a repeated effect key answers the first result).

| Action | When | Params → result |
|---|---|---|
| `cancel_session` | Scheduled | `{reason?}` (cleaned, at most 1000 characters) → `{session, cancelled: [booking ids]}`: the session `Cancelled` with the reason, its waitlist places cancelled first (no freed seat is offered to them), then every seat, each as a business cancellation (a paid deposit back in full); each guest is told once by the next upkeep round |
| `move` | Scheduled (hidden on the record page: the schedule offers it) | `{start_time (RFC 3339 text, ahead), staff?, resource?}` (empty = the same) → `{session, moved}`: decided once for the whole session under the old and new teachers' locks (another booking or session in the way, buffers included, or a taken room: 409; outside hours, time off, the past: 422) and nothing changes on a refusal; the session and every seat move together, keeping the key, each seat told once; waitlist places move silently. Where it already is: `moved: 0`, nobody told |
| `repeat` | Scheduled, not in a series | `{every: week / 2 weeks / 4 weeks / month, count: 2–52 (this one included), skip_conflicts?: "Yes"}` → `{series, scheduled: [{id, start_time}], skipped: [{date, start_time, reason}]}`: later sessions at the same local time in the teacher's zone, each checked as one scheduled by hand; a taken date stops the whole repeat (naming it) unless skipping |

### Waitlists

A full class session takes waitlist places (`join_waitlist` / `waitlist: true`). When seats
free up (a cancellation, a reschedule away or to fewer seats, a larger `capacity`), waiting
bookings move up first come, first served — each whose seats fit — and become `Confirmed`
(`Pending` when public bookings need approval); the guest is told. Places of classes that have
started are cancelled by maintenance ("No seat became free before the class started.").

## Other actions

| Entity | Action | Who | Result |
|---|---|---|---|
| `BookingService` | `find_slots` `{from?, to?, time_zone?, staff?, seats?, booking?}` | read access | Bookable starts from now on (no advance limit), same shape as `GET /slots`; `booking` is a booking being moved, whose own time counts as free |
| `BookingStaff` | `calendar_feed` | the staff member or a manager | `{url, path, expires_at, note}`: their private feed link, valid a year |
| `BookingStaff` | `rotate_calendar_feed` | the staff member or a manager | A new link; earlier ones stop working |

## Public endpoints — `/api/site/{tenant}/bookings/...`

No authentication; the same routes answer signed-in users under `/api/app/bookings/...`.
JSON bodies up to 64 KB, JSON responses with `Cache-Control: no-store`. With the public page
off every route below except the feed answers 404. Unknown, expired and foreign tokens all
answer the same 404. `POST`s are limited to 30 per 10 minutes per address and workspace (429).

| Method & path | Request | Response |
|---|---|---|
| `GET /services` | — | `{time_zone, currency, locations: [{id, name, address, time_zone}], services: [{id, name, description, duration_minutes, seats, price, location_type, location (in person only), locations: [ids it is offered at], staff: [{id, name, bio, location}], questions: [{question, type, options, required}]}]}` — active services with active staff; `locations` are the active ones (empty when the workspace has none) |
| `GET /slots` | `service` (required), `from`, `to` (dates in the zone; default the next 7 days, at most 62), `time_zone` (IANA; default the workspace's), `staff`, `seats`, `location` (resolved as "Locations" below; 422 on `location` when it must be chosen or is not offered), `waitlist` (1: list full classes too, marked `full: true` with `seats_left` what is free, so a page can offer their waitlist) | `{service: {id, name, duration_minutes, seats}, time_zone, from, to, days: [{date, slots: [{start, end, time ("HH:MM" in the zone), seats_left, staff: [{id, name}]}]}]}`. 404 for an unknown or inactive service; 422 for bad parameters |
| `POST /book` | `{service, start_time, staff? (id or "any"), location?, seats?, name, email, phone?, time_zone?, language?, notes?, answers? ({question: answer} or [{question, answer}]), waitlist?, website}` | `201 {booking: View, token}`. `website` is a honeypot: when filled the answer is `201 {"status": "received"}` and nothing is booked. Answers are checked against the questions (required ones present, numbers, Yes/No, one of the choices, nothing unknown). 422 with field errors, 409 when taken, 404 unknown service |
| `GET /manage/{token}` | — | `{booking: View}` |
| `GET /manage/{token}/slots` | `from`, `to`, `time_zone`, `staff` | Like `/slots` for the booking's service, seats and location; its own time does not block |
| `POST /manage/{token}/reschedule` | `{start_time, staff?, seats?}` | `{booking: View}`; guest rules (grid, notice, advance) and the reschedule window; 409 when taken |
| `POST /manage/{token}/cancel` | `{reason?}` | `{booking: View}`; the cancellation window applies (not to waitlist places) |
| `GET /manage/{token}/booking.ics` | — | `text/calendar` (METHOD:PUBLISH), as an attachment |
| `GET /feeds/{staff}.ics` | `v`, `exp`, `sig` (from `calendar_feed`) | `text/calendar`: the staff member's bookings from 30 days ago to a year ahead, a class session as one event with its guests; 404 when the link is wrong, rotated or expired. Works with the public page off |

**View**: `{number, status, service: {id, name}, staff: {id, name}, start_time, end_time,
time_zone, when ("Mon 2 Nov 2026, 10:00 (Asia/Kolkata)"), until ("11:00"), seats,
location_type, location (null when cancelled), place ({id, name, address} of its location, or null), guest: {name, email, phone}, answers: [{question,
answer}], amount, cancellation_reason, waitlist_position (Waitlisted only), can_cancel,
cancel_until, can_reschedule, reschedule_until, links: {manage, ics}, deposit}`.

`deposit` is `null` without Booking Deposits or for a booking with no deposit request, else `{amount, currency,
status (the payment request's: Initiated, Paid, Failed, Cancelled or Expired), payment_url, kept, pay_by}`.
`payment_url` is given only while `status` is `Initiated`, and is null until the payment gateway has written it (a
moment after booking), so a page asks again. `pay_by` is, while `status` is `Initiated`, for a `Pending` booking the
earlier of the request's expiry and the end of the hold (the service's or the settings' `bpay_hold_minutes` from
booking), for an `Expired` booking the request's expiry, else null.

Guest tokens are 256 random bits; only their SHA-256 is stored (`bookings_token`). A booking has
one per link handed out (the booking response, each email, each `guest_link`); all work until
30 days after the booking ends.

## Notifications and reminders

Every guest-visible change bumps the booking's `notice_version`; maintenance turns each version
into one notice, claiming it first in `bookings_notice` in the same transaction that sends, so
nothing is ever sent twice. The first notice describes the booking as it is then (a booking
made and moved before maintenance ran is announced once; one cancelled before that is never
announced).

* **Guests**, by email when the booking has an address. With the Mail app (`comms`) installed a
  `Communication` (Email, Sent, referencing the booking); without it the server's own mailer sends
  the same words through the platform's message log (`message_send`). Each mail has a fresh
  self-service link and an `.ics` link — booking request received (Pending), deposit due (with
  Booking Deposits: how much, until when the time is held, and the guest's own page as the link
  to pay), confirmed, waitlisted, moved, cancelled, released (its deposit was not paid in time;
  the pay link is left out once the deposit can no longer be paid), deposit refunded, and
  reminders. With the Mail app, confirmations and moves carry a calendar invitation
  (METHOD:REQUEST, SEQUENCE = notice version), cancellations a METHOD:CANCEL; the platform's
  mailer sends no calendar part, so without the Mail app the mail has the link only. A workspace
  with neither a mailer nor the Mail app hands out `guest_link`s.
  A Video service with no link of its own (Location details empty) gets a meeting per booking
  when Comms 0.14.0 and a meeting server are there: the guest's join link is the booking's
  location, so every notice and the invitation carry it (see `src/video.rs`).
* **Staff**, in-app: new booking, to confirm, moved, cancelled, waitlist seat filled, deposit not
  paid, an outside-calendar clash — unless they made the change themselves.
* **Wording.** Every message is a message template of the platform, edited, reset and translated
  under Settings, Message templates. Guests: `bookings.guest_requested`, `guest_deposit_due`,
  `guest_confirmed`, `guest_waitlisted`, `guest_rescheduled`, `guest_cancelled`, `guest_reminder`,
  `guest_expired`, `guest_refunded`. Staff: `bookings.staff_new`, `staff_awaits_confirmation`,
  `staff_rescheduled`, `staff_cancelled`, `staff_seat_filled`, `staff_clash`, `staff_expired`.
  Times arrive in the template as text in the guest's or staff member's zone (`values.when`,
  `values.until`). The booking page sends the language it is shown in (`language`, optional, in
  `POST /book`); it is kept as `Booking.guest_language` (erased with the guest) and the guest's
  messages come in it when the template has it, else in the workspace's language. A badly written
  tag is dropped and never refuses a booking.
* **Staff reminders**: with `remind_staff` (Booking Settings, "Remind Staff Too", off until a
  manager switches it on) the booked staff member gets an in-app notice (`bookings.staff_reminder`)
  at each reminder time: once per appointment, once per class session ("Coming up: Yoga (4
  booked)"), at the same offsets as the guest's, also when they made the booking themselves. A
  moved booking is reminded again at its new time. A staff member's mailbox gets it if they chose
  that for their notices.
* **Reminders** (Confirmed bookings, at each `reminders` offset): skipped when already due when
  the booking was made or moved; when several are due at once (maintenance was down) only the
  nearest is sent and the others are claimed as skipped.

## Reports

`GET /api/reports/{name}`; reading Booking is required and the caller's row scope applies (staff
see their own numbers). Periods are workspace-local days (`from_date`, `to_date`; default the
fiscal year to date).

| Report | Rows | Columns |
|---|---|---|
| `bookings_summary` | `group_by`: Service, Staff, Status, Source, Day, Month | bookings, seats taken, pending, confirmed, completed, no-show, cancelled, waitlisted, booked amount (held bookings), invoiced |
| `bookings_attendance` | `group_by`: Service, Staff, Month | bookings (not waitlisted), completed, no-show, cancelled, cancellation rate % (of bookings), no-show rate % (of completed + no-show) |
| `bookings_staff_utilization` | staff (Bookings Users: themselves) | available minutes (working time less time off), booked minutes (each session once), free minutes, sessions, utilization % |

## Background work

Every minute (`maintenance`, one replica at a time), per workspace with the package: send due
change notices (up to 200) and reminders, cancel waitlist places of started classes, promote
waitlists whose sessions have room (capacity raised, a promotion skipped because the row was
busy), purge guest tokens of bookings over for 30 days and claims of bookings over for 60 days.
`maintenance_at(kernel, now)` runs the same as of another time (tests).

## Tables

Besides the entity tables: `bookings_token` and `bookings_notice` (row-level security like every
table), the exclusion constraint `bookings_no_double_booking` and the check
`bookings_active_is_scheduled` on `e_booking` (`sql/001_bookings.sql`); the exclusion constraint
`bookings_sessions_do_not_overlap`, the check `bookings_session_is_scheduled` and the unique key
`bookings_session_key` on `e_booking_session` (`sql/002_sessions.sql`). A booking and a session
cannot share one constraint: both lock the teacher's `e_booking_staff` row first.

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
   | `public_url` | — | This server's internet address, for absolute links in emails and feed URLs |
   | `manage_page_url` | — | Your booking page; guest links become `<url>?token=<token>` (else they point at `/manage/{token}` below) |
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
   | `staff` | Rows `{staff}` — who provides it (at least one) |
   | `resources` | Rows `{resource}` — each booking needs one of them with a free unit (empty = none) |
   | `questions` | Intake questions `{question, answer_type: Text / Long Text / Number / Yes/No / Choice, options (one per line, for Choice), required}` |

## How availability works

* The appointment `[start, start + duration)` lies inside the staff member's working time (their
  zone; DST handled: a skipped local time means the first instant after the gap, a repeated one
  its first occurrence) and outside their time off.
* Each booking blocks `[start − buffer before, end + buffer after)`. Blocks of different
  sessions of one staff member never overlap — the engine checks it, and the database enforces
  it with an exclusion constraint (`bookings_no_double_booking`). Buffers may reach outside
  working hours.
* Seats of a class session (same service, staff member and start) share one block; a session is
  full at `capacity` seats. Existing sessions are offered at their own start even off the grid.
* A resource hosts at most `capacity` sessions at any instant; joining a class uses the
  session's resource. The least loaded eligible resource is taken.
* Guests (public endpoints) book only offered starts — on the grid from the start of each
  working interval — inside the notice and advance windows. Staff (internal bookings, the
  generic API) may book any minute inside working hours, past or future.
* "Any staff" (`staff` empty): join an existing class session first, otherwise the staff member
  with the fewest sessions that local day, then the fewest booked minutes, then a stable order.
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
settings) and guest details from the customer. Staff bookings are `Confirmed` at once; a full
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
| `cancel_booking` | Pending, Confirmed, Waitlisted | `{reason?}` → the booking, `Cancelled`; a freed class seat goes to the waitlist |
| `complete` | Confirmed, once started | → `Completed` |
| `mark_no_show` | Pending, Confirmed, once started | → `No Show` |
| `make_sales_invoice` | Confirmed, Completed, No Show | → `{entity: "SalesInvoice", id, number}`: a draft invoice of the booking's company in its currency, one line `item × seats` at the booked `rate`, no discount (the company's default sales taxes apply). A guest without a customer becomes one (an enabled customer with the guest's email, else a new individual). Refused while a draft or submitted invoice exists; a deleted draft (the link clears) or a cancelled invoice allows a new one. Needs Sales Invoice create permission |
| `guest_link` | not Cancelled | → `{token, url, api, expires_at, note}`: a fresh self-service link to hand to the guest (the fallback without the Mail app) |

### Waitlists

A full class session takes waitlist places (`join_waitlist` / `waitlist: true`). When seats
free up (a cancellation, a reschedule away or to fewer seats, a larger `capacity`), waiting
bookings move up first come, first served — each whose seats fit — and become `Confirmed`
(`Pending` when public bookings need approval); the guest is told. Places of classes that have
started are cancelled by maintenance ("No seat became free before the class started.").

## Other actions

| Entity | Action | Who | Result |
|---|---|---|---|
| `BookingService` | `find_slots` `{from?, to?, time_zone?, staff?, seats?}` | read access | Bookable starts from now on (no advance limit), same shape as `GET /slots` |
| `BookingStaff` | `calendar_feed` | the staff member or a manager | `{url, path, expires_at, note}`: their private feed link, valid a year |
| `BookingStaff` | `rotate_calendar_feed` | the staff member or a manager | A new link; earlier ones stop working |

## Public endpoints — `/api/site/{tenant}/bookings/...`

No authentication; the same routes answer signed-in users under `/api/app/bookings/...`.
JSON bodies up to 64 KB, JSON responses with `Cache-Control: no-store`. With the public page
off every route below except the feed answers 404. Unknown, expired and foreign tokens all
answer the same 404. `POST`s are limited to 30 per 10 minutes per address and workspace (429).

| Method & path | Request | Response |
|---|---|---|
| `GET /services` | — | `{time_zone, currency, services: [{id, name, description, duration_minutes, seats, price, location_type, location (in person only), staff: [{id, name, bio}], questions: [{question, type, options, required}]}]}` — active services with active staff |
| `GET /slots` | `service` (required), `from`, `to` (dates in the zone; default the next 7 days, at most 62), `time_zone` (IANA; default the workspace's), `staff`, `seats` | `{service: {id, name, duration_minutes, seats}, time_zone, from, to, days: [{date, slots: [{start, end, time ("HH:MM" in the zone), seats_left, staff: [{id, name}]}]}]}`. 404 for an unknown or inactive service; 422 for bad parameters |
| `POST /book` | `{service, start_time, staff? (id or "any"), seats?, name, email, phone?, time_zone?, notes?, answers? ({question: answer} or [{question, answer}]), waitlist?, website}` | `201 {booking: View, token}`. `website` is a honeypot: when filled the answer is `201 {"status": "received"}` and nothing is booked. Answers are checked against the questions (required ones present, numbers, Yes/No, one of the choices, nothing unknown). 422 with field errors, 409 when taken, 404 unknown service |
| `GET /manage/{token}` | — | `{booking: View}` |
| `GET /manage/{token}/slots` | `from`, `to`, `time_zone`, `staff` | Like `/slots` for the booking's service and seats; its own time does not block |
| `POST /manage/{token}/reschedule` | `{start_time, staff?, seats?}` | `{booking: View}`; guest rules (grid, notice, advance) and the reschedule window; 409 when taken |
| `POST /manage/{token}/cancel` | `{reason?}` | `{booking: View}`; the cancellation window applies (not to waitlist places) |
| `GET /manage/{token}/booking.ics` | — | `text/calendar` (METHOD:PUBLISH), as an attachment |
| `GET /feeds/{staff}.ics` | `v`, `exp`, `sig` (from `calendar_feed`) | `text/calendar`: the staff member's bookings from 30 days ago to a year ahead, a class session as one event with its guests; 404 when the link is wrong, rotated or expired. Works with the public page off |

**View**: `{number, status, service: {id, name}, staff: {id, name}, start_time, end_time,
time_zone, when ("Mon 2 Nov 2026, 10:00 (Asia/Kolkata)"), until ("11:00"), seats,
location_type, location (null when cancelled), guest: {name, email, phone}, answers: [{question,
answer}], amount, cancellation_reason, waitlist_position (Waitlisted only), can_cancel,
cancel_until, can_reschedule, reschedule_until, links: {manage, ics}}`.

Guest tokens are 256 random bits; only their SHA-256 is stored (`bookings_token`). A booking has
one per link handed out (the booking response, each email, each `guest_link`); all work until
30 days after the booking ends.

## Notifications and reminders

Every guest-visible change bumps the booking's `notice_version`; maintenance turns each version
into one notice, claiming it first in `bookings_notice` in the same transaction that sends, so
nothing is ever sent twice. The first notice describes the booking as it is then (a booking
made and moved before maintenance ran is announced once; one cancelled before that is never
announced).

* **Guests**, by email when the Mail app (`comms`) is installed and the booking has an email: a
  `Communication` (Email, Sent, referencing the booking) with a fresh self-service link and an
  `.ics` link — booking request received (Pending), confirmed, waitlisted, moved, cancelled, and
  reminders. Confirmations and moves carry a calendar invitation (METHOD:REQUEST, SEQUENCE =
  notice version), cancellations a METHOD:CANCEL. Without the Mail app guests get nothing
  automatically: staff hand out `guest_link`s.
* **Staff**, in-app: new booking, to confirm, moved, cancelled, waitlist seat filled — unless
  they made the change themselves.
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
`bookings_active_is_scheduled` on `e_booking` (`sql/001_bookings.sql`).

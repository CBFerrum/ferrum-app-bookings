<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/banner-dark.svg">
  <img alt="Ferrum Bookings: appointments and classes that book themselves" src="docs/assets/banner-light.svg" width="100%">
</picture>

<br>

[![Version](https://img.shields.io/badge/version-0.7.2-0e7490?style=flat-square)](CHANGELOG.md)
[![Licence: AGPL-3.0](https://img.shields.io/badge/licence-AGPL--3.0-blue?style=flat-square)](LICENSE)
[![Ferrum platform](https://img.shields.io/badge/runs%20on-Ferrum-2f5bea?style=flat-square)](https://github.com/CBFerrum/Ferrum)
![Rust](https://img.shields.io/badge/Rust-server-b7410e?style=flat-square&logo=rust)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-data-336791?style=flat-square&logo=postgresql&logoColor=white)
![React](https://img.shields.io/badge/React-TypeScript-087ea4?style=flat-square&logo=react&logoColor=white)
![Tests](https://img.shields.io/badge/tests-52%20Rust%20%C2%B7%2076%20UI-16a34a?style=flat-square)

### Appointments and classes that book themselves, with no double bookings.

Ferrum Bookings is for clinics, studios, salons, consultants and any team that sells time. It gives guests a public booking page and gives the front desk a schedule, a day list and exact availability. It replaces Calendly, Acuity Scheduling or SimplyBook.me when you want the bookings, the customers and the invoices in one self-hosted system.

[💡 Why](#-why-bookings) · [📸 Screenshots](#-screenshots) · [✨ Features](#-features) · [📊 Compare](#-how-it-compares) · [📍 Roadmap](#-roadmap) · [🚀 Install](#-install) · [🔧 Develop](#-develop)

</div>

## 💡 Why Bookings

- 🧠 **Exact availability.** Working hours are read in each person's own time zone. Daylight saving, buffers, notice windows, time off, rooms and outside calendars all count before a time is offered.
- 🔒 **No double bookings, even under load.** Every booking locks the people and rooms it needs, and the database refuses an overlap with an exclusion constraint.
- 🌐 **A booking page guests can use alone.** Service, person, day and time from what is really free in the guest's zone. They can move, cancel or add the booking to their calendar from the link in their email.
- 🧾 **One data set with the books.** A guest becomes an ERP customer when they book. A paid booking becomes a draft sales invoice, exactly once.
- 🏠 **Self-hosted and open source.** AGPL-3.0, your server and your data. No per-person licence fee.
- 🧩 **Extendable in Studio.** Add fields, rules and reports to any record without code. People and AI agents use the same REST and MCP interfaces, under the same permissions.
- 🌍 **Reads in three languages.** English, Arabic (right to left) and Hindi.

## 🎯 At a glance

| | |
|---|---|
| **Version** | 0.7.2 (Booking Deposits connector 0.2.0) |
| **Kind** | Its own interface at `/bookings/` for the team, and a public booking page at `/bookings/book/{workspace}`. Records open in the platform's records screens |
| **What it adds** | 13 entities (6 you work with, 7 that sit inside them), 2 roles, 3 reports, 1 menu module, 2 packages (`bookings`, `bookings_payments`), 1 payments connector, 4 database migrations |
| **Depends on** | Ferrum platform `>=0.3.2, <0.4` and [ERP](https://github.com/CBFerrum/ferrum-app-erp) `>=0.1` (installing Bookings installs ERP) |
| **Works with** | Mail, Chat and Calendar (Comms) for emails, calendar invitations and video meetings, Commerce for deposits, ERP for customers, items and invoices, platform working calendars for holidays |
| **Languages** | English, Arabic, Hindi (about 380 interface words each, and 250 names each) |
| **Tests** | 52 Rust tests, 76 interface tests |
| **Licence** | AGPL-3.0-only |

## 📸 Screenshots

<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/bookings-schedule-dark.webp"><img alt="Bookings — schedule" src="docs/assets/screens/bookings-schedule-light.webp"></picture><br><b>🗓️ The schedule</b><br><sub>See each person's day, click an empty time to book it, and drag a booking to move it.</sub></p>

<table>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/bookings-today-dark.webp"><img alt="Bookings — today" src="docs/assets/screens/bookings-today-light.webp"></picture>
<p><b>✅ Today's list</b><br><sub>Confirm what waits, and mark who came, from one list for the front desk.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/bookings-booking-page-dark.webp"><img alt="Bookings — booking page" src="docs/assets/screens/bookings-booking-page-light.webp"></picture>
<p><b>🌐 Booking page settings</b><br><sub>Open the public page, set notice, how far ahead guests book, and when they can still cancel or move.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/bookings-booking-list-dark.webp"><img alt="Bookings — list" src="docs/assets/screens/bookings-booking-list-light.webp"></picture>
<p><b>📋 All bookings</b><br><sub>Search and filter every booking by status, service or person.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/bookings-booking-service-list-dark.webp"><img alt="Booking services — list" src="docs/assets/screens/bookings-booking-service-list-light.webp"></picture>
<p><b>🧘 Services</b><br><sub>Appointments and classes with duration, seats, price and where they happen.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/bookings-booking-staff-list-dark.webp"><img alt="Booking staff — list" src="docs/assets/screens/bookings-booking-staff-list-light.webp"></picture>
<p><b>👥 Staff</b><br><sub>People guests book, each in their own time zone.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/bookings-booking-new-dark.webp"><img alt="Booking — new" src="docs/assets/screens/bookings-booking-new-light.webp"></picture>
<p><b>➕ New booking</b><br><sub>Book a guest by hand: service, person, time and their details.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/bookings-report-bookings-summary-dark.webp"><img alt="Bookings summary — report" src="docs/assets/screens/bookings-report-bookings-summary-light.webp"></picture>
<p><b>📊 Bookings summary</b><br><sub>Count bookings, seats and booked amounts by service, person, status or month.</sub></p>
</td>
<td width="50%"></td>
</tr>
</table>

<p align="center">
<span><picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/bookings-schedule-phone-dark.webp"><img alt="Bookings — schedule" src="docs/assets/screens/bookings-schedule-phone-light.webp" width="260"></picture><br><sub><b>🗓️ Schedule on a phone</b></sub></span>&nbsp;
<span><picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/bookings-booking-list-phone-dark.webp"><img alt="Bookings — list" src="docs/assets/screens/bookings-booking-list-phone-light.webp" width="260"></picture><br><sub><b>📋 Bookings on a phone</b></sub></span>&nbsp;
</p>

## ✨ Features

### 🗓️ Services and classes

- **Appointments and group classes.** A service is one seat (an appointment) or many seats per session (a class). Seats are set per service, from 1 to 1000.
- **Time and buffers.** Duration from 5 minutes to 24 hours, a buffer kept free before and another after each booking, and a slot interval that decides which start times are offered. Each service may override the workspace's interval.
- **Price and item.** A price per seat in the company currency. A priced service names the ERP item it is invoiced as.
- **Where it happens.** In person, by video or by phone, with an address, link or number that only a booked guest sees.
- **Intake questions.** Ask text, long text, number, yes or no, or a choice from a list. Questions can be required, and the server checks every answer against its question.
- **Rules per service.** Minimum notice, how far ahead guests book, cancel and reschedule windows, manual confirmation and reminders can be set on one service. An empty field follows Booking Settings.
- **Staff and resources per service.** Choose who provides it and which rooms or equipment it needs.
- **Series on the page.** A service can let a guest book it every week, every 2 weeks, every 4 weeks or every month, up to 52 times, at the same local time (free services only).

### 👥 Staff, time and availability

- **Staff in their own time zone.** Hours are entered as local time in an IANA zone. Several intervals a day leave breaks, and a night shift runs past midnight.
- **Working calendars.** A staff member, a resource or the whole business may follow a platform working calendar (Admin, Calendars). Its weekends, holidays and dated exceptions close or open time for everyone who follows it, also on a day the clocks change. A Saturday opened by an exception is offered; a closed afternoon is not.
- **Time off.** A person enters their own time off. The record counts the active bookings already inside it, so nobody is surprised.
- **Outside calendars.** A staff member can give the iCalendar address of a calendar they keep elsewhere (Google, Outlook, iCloud, Nextcloud, Ferrum Comms). It is read every quarter of an hour, and guests are not offered the times it says are busy. Only times are kept. Recurring events, exceptions and moved occurrences are read correctly.
- **Clash marks.** A booking that an outside calendar later became busy over is marked, the staff member is told, and the guest is offered another time. Keep Despite the Clash leaves it where it is.
- **Fair choice of "any".** When a guest picks nobody in particular, the booking joins an existing class session first, then the person with the fewest sessions that day, then the fewest booked minutes.
- **Rooms and equipment.** A resource (room, equipment, vehicle, other) has a capacity. A booking takes the least loaded free one.
- **Leavers.** When a person is removed from the workspace they take no more bookings. Their existing bookings stay for a person to move.
- **Calendar feed per person.** A private, signed feed link (valid a year) puts a person's bookings in any calendar app. A class session is one event with its guests. The link can be reset.

### 🌐 The public booking page

- **Step by step, in the address.** Service, person when it matters, day, time, details, confirm. Back goes back a step, and a link can start anywhere, for example `?service=`.
- **Times in the guest's zone.** The guest can change the zone. Times come from what is really free, not from a fixed grid.
- **Built for phones.** The page works the same on a small screen.
- **If it was taken while they typed.** Confirm is disabled with a message, and the server's complaints sit beside the field.
- **Waitlist for a full class.** A guest can join the waitlist instead of failing.
- **Their own booking page.** Every link shows the booking, lets the guest add it to a calendar (`.ics`), move it or cancel it. A cancel or move is allowed up to the configured number of hours before the start.
- **Guests become customers.** A booking finds the guest's ERP customer by email or phone digits, or makes one, when it is booked.
- **Spam guard.** A hidden field and a limit of 30 writes per 10 minutes per address and workspace.
- **Your own page.** Booking Settings can point guest links at a page of your own instead of the app's page.
- **Switch off.** With the public page off, every public address answers not found. Staff calendar feeds keep working.

### 📅 The schedule and today's list

- **A day by person, room or a week.** One column per person (or room, or day). Bookings that need an answer sit in a strip above.
- **Click to book, drag to move.** Click an empty time to book it. Drag a booking, or press M and the arrow keys, to move it. A move shows at once and goes back with the server's reason if refused.
- **A sheet per booking.** Open a booking or a class session (one row with its seats) to confirm, move, cancel, mark and invoice.
- **Shareable view.** Day, date and filters live in the address, so a link shows what the sender saw.
- **Today's list.** Ordered as the day will happen: what needs an answer, what is under way, what comes next, and what has ended and still needs closing (did they come?). Staff see their own bookings by default; managers see everyone's.
- **Marks.** A booking whose time a working calendar closed shows **Outside opening hours**. One that an outside calendar clashes with shows the clash mark. Expired bookings are muted and hold no block.
- **Time off on the schedule.** Enter time off from the schedule, where its conflicts show.

### ✅ Booking life cycle

| State | Meaning |
|---|---|
| **Pending** | Waiting for a person to confirm, or for the deposit. Holds its time and seats |
| **Confirmed** | Holds its time and seats |
| **Waitlisted** | A full class. First come, first served when a seat frees |
| **Expired** | The deposit was not paid in time. Its time is released and the guest told |
| **Cancelled** | By the guest, or by staff with a reason the guest sees |
| **No Show** | Marked after the start. A deposit, if any, is kept |
| **Completed** | Marked after the start |

- **Actions.** Confirm, Reschedule, Cancel Booking (with a reason), Mark Completed, Mark No Show, Create Sales Invoice, Guest Self-Service Link, Repeat, Cancel This and Later, Keep Despite the Clash.
- **Staff bookings** are confirmed at once. Public bookings wait as Pending when confirmation is manual.
- **Times change only through Reschedule,** which re-checks everything like a new booking. Statuses change only through the actions.
- **Active bookings cannot be deleted.** Cancel them.
- **Numbering.** `BKG-{year}-{5 digits}`.
- **Series.** Repeat a booking every week, 2 weeks, 4 weeks or month, 2 to 52 times in all. Taken times either stop the series or are skipped. Cancel This and Later ends the rest.
- **Waitlists.** Freed seats (a cancellation, a move away, fewer seats, a larger capacity) go to the first waiting booking whose seats fit, which is confirmed and told. Places of classes that have started are cancelled.

### 💳 Money

- **Invoices into ERP.** Create Sales Invoice makes one draft invoice for the booking's company: one line of the item times the seats at the booked rate. It is refused while another live invoice exists, and a guest without a customer becomes one.
- **Deposits (Booking Deposits, with Commerce).** A service may ask a percentage of its price (100 for in full) when a guest books. The booking waits as Pending, is invoiced at once, and is confirmed when the deposit is paid through the store's payment method.
- **Unpaid deposits release the time.** After the configured minutes, or when the payment request expires, the booking expires and the guest is told. Paid late, it is restored if the time is still free, and refunded in full if it was taken.
- **Refunds by terms.** Cancelled early enough, the deposit comes back whole. Cancelled later, the service keeps its share. The invoice is credited for what is not kept, and the guest's refund goes back through the gateway once a person approves it. A no-show keeps the deposit.

### 🔔 Messages and reminders

- **One notice per change.** Every guest-visible change becomes exactly one notice, claimed in the same transaction that sends it, so nothing is sent twice. A booking made and moved before the next run is announced once, and one cancelled before then is never announced.
- **Guest emails** (with the Mail app): request received, confirmed, waitlisted, moved, cancelled, and reminders. Each carries a fresh self-service link and a calendar invitation (a request for confirmations and moves, a cancel for cancellations).
- **Reminders** before confirmed bookings, at each offset set (for example a day and an hour). A reminder already due when the booking was made or moved is skipped.
- **Staff notices** in the app: new booking, one to confirm, moved, cancelled, a waitlist seat filled, an outside-calendar clash. A person is not told of their own changes.
- **Without the Mail app** guests get nothing automatically; staff hand out the Guest Self-Service Link.
- **Video meetings.** A Video service with no link of its own gets a meeting for each booking in Ferrum Comms once confirmed, hosted by the booked staff member. The guest's join link needs no account and is in the booking, confirmation, reminders and calendar entry. Moving a booking moves its meeting. Cancelling ends it. A meeting that cannot be made never stops a booking.

### 📊 Reports

| Report | Rows | Columns |
|---|---|---|
| **Bookings Summary** | Service, Staff, Status, Source, Day or Month | Bookings, seats taken, pending, confirmed, completed, no-show, cancelled, waitlisted, booked amount, invoiced |
| **Bookings Attendance** | Service, Staff or Month | Bookings, completed, no-show, cancelled, cancellation rate, no-show rate |
| **Bookings Staff Utilization** | Staff (a Bookings User sees themselves) | Available minutes (working time less time off), booked minutes, free minutes, sessions, utilization % |

Reports follow each person's row scope, run over workspace-local days, and default to the fiscal year to date.

### 🔐 Permissions, privacy and APIs

- **Two roles.** Bookings Manager runs everything. Bookings User sees and works their own bookings (assigned to them or made by them) and their own time off.
- **Row-level security.** Staff without a Bookings role see nothing of the app.
- **Guest links are safe.** A link holds 256 random bits and only its SHA-256 is stored. Unknown, expired and foreign links all answer the same way. A link works until 30 days after the booking ends.
- **Personal data.** An export or erasure request finds bookings by email or phone and lists the ones holding the person's name apart. Erasure clears name, email, phone, answers, notes and the cancellation reason. The time the booking held stays.
- **Records for free.** Lists, forms, search, imports (services and bookings), exports, audit trail, REST and MCP come from the platform.
- **Public endpoints.** `/api/site/{workspace}/bookings/` serves services, free slots, booking, and a guest's own booking (view, slots, move, cancel, `.ics`). Times are UTC with milliseconds, and errors use the platform's usual shape.
- **The assistant** knows what the records mean and suggests questions in the app's pages.

### 🌍 Languages and accessibility

- **English, Arabic and Hindi** for the app's words, record names and report names. Pages mirror right to left. The Arabic and Hindi text was translated by a model and is waiting for review by native speakers.
- **Keyboard moves.** A booking can be moved on the schedule from the keyboard.
- **Honest states.** Loading, empty, no matches, denied, offline and failed are told apart.

## 🧭 How it works

```mermaid
flowchart LR
    G["Guest picks a service"] --> S["Free slots in the guest's zone"]
    S --> B["Lock, check, insert"]
    B -->|"free"| C["Confirmed or Pending"]
    B -->|"full class"| W["Waitlisted"]
    C --> R["Reminders and calendar invitation"]
    C --> D["Done: Completed or No Show"]
    C --> X["Cancelled by guest or staff"]
    X --> P["Waitlist promoted"]
    D --> I["Draft sales invoice in ERP"]
```

A booking is checked against working hours and calendars, time off, outside calendars, other bookings with their buffers, seats and rooms. The check and the insert happen under row locks, and the database enforces the rule once more.

```mermaid
flowchart LR
    BK["Bookings"] --> ERP["ERP: customers, items, invoices"]
    BK --> COM["Comms: email, invitations, video"]
    BK --> CAL["Platform working calendars"]
    BK --> OUT["Outside calendars (iCalendar)"]
    BK --> COMM["Commerce: deposits and refunds"]
    BK --> OB["Outbox: webhooks and notifications"]
```

Everything Bookings adds is ordinary Ferrum metadata and Rust. Its records get lists, forms, search, permissions, an audit trail, exports and the REST and MCP APIs from the platform. A background job runs every minute for each workspace: it sends due notices and reminders, cancels waitlist places of classes that started, promotes waitlists, moves old dated openings into the calendars, reads outside calendars, expires unpaid deposits, and purges old guest links and claims.

## 🧱 Data model

| Entity | What it is | Key fields or states |
|---|---|---|
| **Booking** | An appointment or a seat in a class | Service, staff, start and end, seats, status, source (Internal or Public), series, resource, customer, guest details, intake answers, amount, sales invoice, clash marks |
| **Booking Service** | What guests book | Duration, buffers, interval, seats, price and item, location, staff, resources, intake questions, rules |
| **Booking Staff** | Someone guests book | Display name, email, IANA time zone, working hours, working calendar, outside calendar address, bio |
| **Staff Time Off** | Time a person cannot be booked | Staff, from, to, reason, conflicting bookings |
| **Booking Resource** | A room or equipment | Type, capacity, working calendar, active |
| **Booking Settings** | One per workspace | Public page, manual confirmation, notice, advance, interval, windows, reminders, addresses, business calendar |
| **Working Hours, Service Staff, Service Resource, Intake Question, Intake Answer, Booking Reminder, Calendar Date** | Rows inside the records above | Weekday and times, who, which room, question and type, answer, minutes before, a dated closing or opening (superseded by working calendars) |

Deposits add fields to Booking Service, Booking Settings and Booking (deposit percent, refund hours, hold minutes, kept percent, payment method, deposit, payment request, refund).

| Role | What it can do |
|---|---|
| **Bookings Manager** | Everything: services, staff, hours, time off, resources, settings and every booking; reads and edits customers; creates draft sales invoices |
| **Bookings User** | Reads services, staff, resources and settings; works their own bookings; enters their own time off; gets their own calendar feed |

Reports: Bookings Summary, Bookings Attendance, Bookings Staff Utilization. There are no print formats or workflows beyond the booking states above.

## 🔌 Integrations

| With | How | Needs |
|---|---|---|
| **ERP** | Customers, items, companies and draft sales invoices | ERP installed (done for you) |
| **Comms (Mail, Chat and Calendar)** | Guest emails, calendar invitations, video meetings | The Mail app, and for video a meeting server and the workspace's public address |
| **Commerce** | Deposits through the store's payment methods, and refunds | Commerce `>=0.7`, Booking Deposits, a payment gateway plugin |
| **Working calendars** | Holidays and dated exceptions for staff, resources and the business | Created in Admin, Calendars |
| **iCalendar** | Read a staff member's outside calendar (Google, Outlook, iCloud, Nextcloud); give out a signed feed per person; `.ics` per booking | The calendar's secret address |
| **Webhooks and the outbox** | Record events from the platform | A webhook endpoint |
| **REST and MCP** | Every record and action, within each person's permissions | An API token |
| **Personal-data requests** | Export and erase a guest's booking data | None |

## 📊 How it compares

Leaders: Calendly, Acuity Scheduling, Microsoft Bookings, SimplyBook.me and Cal.com (open source).

| Capability | Ferrum Bookings | Calendly | Acuity | Microsoft Bookings | SimplyBook.me | Cal.com |
|---|:-:|:-:|:-:|:-:|:-:|:-:|
| Self-hosted, you hold the data | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ |
| Open-source licence | ✅ | ❌ | ❌ | ❌ | ❌ | 🟡¹ |
| No per-person licence fee | ✅ | ❌ | ❌ | 🟡² | ❌ | ✅ |
| Invoices land in the company's own books | ✅ | 🟡³ | 🟡³ | 🟡³ | 🟡³ | 🟡³ |
| Public booking page | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Guest sees times in their own zone | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Services with duration, buffers and notice | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Rules that differ per service | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Group classes with seats | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Class timetable with its own sessions | 🟡⁴ | ❌ | ✅ | ❌ | ✅ | 🟡 |
| Holidays and dated exceptions | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Any available person, chosen fairly | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Weighted round robin or priorities | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ |
| Rooms and equipment with capacity | ✅ | ❌ | 🟡 | ❌ | ✅ | ❌ |
| Intake questions | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Guest cancels or moves from a link | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Guest books a series | 🟡⁵ | ❌ | ✅ | 🟡 | ✅ | ✅ |
| Pay a deposit when booking | 🟡⁶ | ✅ | ✅ | 🟡 | ✅ | ✅ |
| Packages, memberships, gift cards | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ |
| Email reminders and invitations | 🟡⁷ | ✅ | ✅ | ✅ | ✅ | ✅ |
| SMS reminders | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Editable message wording | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Booking written to Google or Outlook, two ways | 🟡⁸ | ✅ | ✅ | 🟡 | ✅ | ✅ |
| A video meeting made per booking | 🟡⁹ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Embed on your own website | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Your logo and colours on the page | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Front-desk schedule with drag to move | ✅ | ❌ | ✅ | 🟡 | ✅ | ❌ |
| Mark who came, no-show rates | ✅ | ✅ | ✅ | 🟡 | ✅ | ✅ |
| Staff utilization report | ✅ | ✅ | ✅ | 🟡 | ✅ | 🟡 |
| Separate locations with their own staff | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ |
| Zapier and Make connectors | ❌ | ✅ | ✅ | 🟡 | ✅ | ✅ |
| API and webhooks | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

<sub>¹ Cal.com has an open core with commercial enterprise features. ² Included in some Microsoft 365 plans. ³ Through integrations or exports, not one data set. ⁴ A class is a service with seats; a session exists once its first seat is booked, and any time inside working hours is offered. ⁵ Free services only. A paid series is booked one at a time. ⁶ Needs Commerce and a payment gateway plugin; refunds are approved by a person. ⁷ Needs the Mail app. ⁸ Bookings reach a calendar by a signed feed, and busy times are read from an iCalendar address every 15 minutes. ⁹ Meetings are made in Ferrum Comms, not Zoom, Meet or Teams.</sub>

### 🏆 Where Bookings stands out

- 🔒 The database itself refuses a double booking, and every change takes row locks in a fixed order.
- 🧾 A booking, its customer and its invoice are one data set with accounting, stock and the rest of Ferrum.
- 🏠 Self-hosted, open source and free of per-person fees, with an open record API and MCP.
- 🕒 DST-correct availability across staff zones, with buffers, calendars, time off and outside calendars.
- 🔔 Exactly-once notices and invoices, and unpaid deposits that release their time.
- 🌍 Arabic and Hindi with right-to-left pages.

### 🚧 Where the leaders are still ahead

- 📣 **Messages.** SMS, WhatsApp and editable wording. Ferrum sends fixed English-text emails (the wording is built in code) and in-app notices.
- 🌐 **The website.** Embed snippets, logo and colours, and a returning-client area.
- 📆 **Calendar sync.** Writing each booking into a Google or Outlook calendar and checking several outside calendars.
- 👥 **Team scheduling.** Collective bookings, weighted round robin and priorities (Calendly, Cal.com).
- 🎟️ **Stored value.** Packages, memberships, gift cards, tips and promo codes (Acuity, SimplyBook.me).
- 🏢 **Several locations,** each with its own staff, hours and time zone.
- 🔗 **Ready-made connectors** such as Zapier and Make, and booking by search engines and maps.

> [!NOTE]
> Leader columns are from public product information as of November 2026 and may change. Features marked 🟡 for a leader depend on plan, region or add-on.

## 📍 Roadmap

### ✅ Recently shipped

- [x] **0.7.2:** a date written into the old Closed and Extra Open Dates field at any time is moved into the working calendar at the next upkeep.
- [x] **0.7.1:** a platform calendar's dated exceptions decide availability, the Outside opening hours mark and the utilization report.
- [x] **0.7.0:** a video meeting for each booking of a Video service, with the guest's own join link.
- [x] **0.6.2:** the interface shows Expired and Outside opening hours, with Arabic and Hindi names.
- [x] **0.6.0:** working calendars with holidays, rules per service, unpaid deposits that release their time, and guests that become customers at once.
- [x] **0.5.0:** a booking whose staff member's outside calendar became busy is marked, told to both sides and moved or kept.
- [x] **0.4.15:** guests book a series from the booking page, where the service allows it.

### 🔜 Next up

**Messages**
- [ ] Editable wording per event and per service, with the logo, and a switch to turn a message off. Today the text is fixed in code.
- [ ] SMS and WhatsApp reminders and confirmations, through the workspace's own gateway.
- [ ] Several reminders per booking with their own audience, follow-ups and review requests.
- [ ] Staff told by email or push, not only in the app.

**Booking page**
- [ ] An embed snippet (inline, popup, floating button).
- [ ] Logo, colours, cover image and staff photos on the page.
- [ ] A returning-client area: my bookings, rebook, saved details.
- [ ] A bot check beyond the hidden field and the rate limit.

**Calendars**
- [ ] Write each booking to the staff member's Google or Outlook calendar and keep it up to date.
- [ ] Check more than one outside calendar for a person.

**Payments**
- [ ] Fixed-amount deposits, pay later per service, and a cancellation policy the guest accepts.
- [ ] Packages, credits, memberships and gift cards.
- [ ] An invoice made automatically on completion.

**Scheduling**
- [ ] A class timetable of real sessions, with a title, capacity and instructor each, and a substitute.
- [ ] Waitlist for one-to-one slots, and waitlist offers with a claim window.
- [ ] Weighted round robin and priorities, and collective bookings with several hosts.

### 🔭 Later

- [ ] Locations with their own staff, resources, hours and time zone, and reports by location.
- [ ] Service categories, images and colours, duration options and add-ons, several services in one visit.
- [ ] A different price or duration per staff member.
- [ ] Tips, discounts and promo codes.
- [ ] A client history (visits, no-shows, spend), notes, files and reviews after the visit.
- [ ] Check-in and in-service states, self check-in, and completing before the start time.
- [ ] Bulk move of the bookings affected by time off or a leaver, and editing a whole series.
- [ ] Blocking or asking approval for clients who often do not show up.
- [ ] Intake with files, consent and conditional questions.
- [ ] Source and campaign tracking on each booking, and search-engine and map listings.
- [ ] Extra attendees on a one-to-one booking.
- [ ] Paid revenue, new versus returning clients and lead-time reports.
- [ ] An installable staff page with push notices and an offline day view.
- [ ] Zapier and Make connectors, and partner API keys.
- [ ] Staff without a Ferrum login, and a staff import.
- [ ] Approval per service or per person, with an approver who is not the person booked.

## 🚀 Install

Bookings is a native Ferrum app. It ships in the official Ferrum server image, and a workspace administrator installs it from **Apps** (`/store/`). It requires ERP `>=0.1`, and installing it in a workspace without ERP installs ERP whole. Self-hosters who build their own image enable the `bookings` feature of the server.

First steps after installing:

1. Give people the **Bookings Manager** or **Bookings User** role.
2. In **Booking Settings**, set the company bookings are invoiced from, the server address (for links in emails), and your notice, advance and cancellation windows. Turn on **Public Booking Page**.
3. Add **Booking Staff** (the email is the person's Ferrum user), with their time zone and hours, or a working calendar from Admin, Calendars.
4. Add **Booking Resources** if services need a room.
5. Add **Booking Services** with duration, seats, price, staff and any intake questions.
6. Share `/bookings/book/{workspace}`. Install the Mail app so guests get emails.
7. For deposits install **Booking Deposits** (needs Commerce `>=0.7`) and choose a payment method in Booking Settings.

## 🔧 Develop

Clone it next to the platform:

```text
work/
├── ferrum/                  https://github.com/CBFerrum/Ferrum
├── ferrum-app-erp/          optional, for working on both together
└── ferrum-app-bookings/
```

Point the platform dependencies at your checkout (this file is git-ignored):

```toml
# .cargo/config.toml
[patch."https://github.com/CBFerrum/Ferrum"]
ferrum-calc = { path = "../ferrum/crates/calc" }
ferrum-domain = { path = "../ferrum/crates/domain" }
ferrum-kernel = { path = "../ferrum/crates/kernel" }
ferrum-meta = { path = "../ferrum/crates/meta" }
```

The tests need PostgreSQL with an ordinary (non-superuser) role, so row-level security applies:

```bash
createuser ferrum --pwprompt && createdb -O ferrum ferrum_bookings_test
export FERRUM_BOOKINGS_TEST_DATABASE_URL=postgres://ferrum:ferrum@localhost:5432/ferrum_bookings_test
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Its tests also build on [ferrum-app-erp](https://github.com/CBFerrum/ferrum-app-erp), which Cargo takes from GitHub. To work on it alongside this app, add:

```toml
[patch."https://github.com/CBFerrum/ferrum-app-erp"]
ferrum-app-erp = { path = "../ferrum-app-erp" }
```

The interface (`ui/`) is built with the platform's packages from the sibling `ferrum/web` checkout (Ferrum-Frontend), and its built files (`ui/dist`) are committed and embedded in the crate. After changing `ui/src`:

```bash
cd ui
npm run test && npm run typecheck && npm run build && npm run check
```

## 📂 Project layout

```text
app.yaml          the manifest: id, version, requirements, what it adds to Ferrum
packages/
  bookings/       entities, roles, reports, menu, SQL migrations, a package guide
  bookings_payments/  deposit fields added to Booking, Service and Settings
connectors/payments/  Booking Deposits, offered where Commerce is installed
src/              the logic in Rust: availability engine, booking, series, notices,
                  outside calendars, ics, video, deposits, reports, public endpoints
ui/               the team's screens and the public booking page (React, TypeScript)
tests/            integration tests against a real PostgreSQL
CHANGELOG.md      what changed in each version
```

## 📜 Changelog

Every version is in [CHANGELOG.md](CHANGELOG.md).

## 🤝 Contributing

Issues and pull requests are welcome at [github.com/CBFerrum/ferrum-app-bookings](https://github.com/CBFerrum/ferrum-app-bookings/issues). Run the checks in Develop first.

## 📄 License

[AGPL-3.0-only](LICENSE), like the platform.

<div align="center">

<sub>Part of <a href="https://github.com/CBFerrum/Ferrum">Ferrum</a>, the open business platform · Made with 🦀 Rust and 🐘 PostgreSQL</sub>

</div>

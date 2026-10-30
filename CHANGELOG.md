# Changelog

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

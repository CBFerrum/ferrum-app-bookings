# Changelog

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

# Ferrum Bookings

A native app for [Ferrum](https://github.com/CBFerrum/Ferrum), the open-source, AI-first ERP.

- Services as one-to-one appointments or group classes: duration, buffers before and after, price and invoicing item, seats, in-person / video / phone locations, intake questions
- Staff with their own IANA time zone and several working intervals a day, time off, and rooms or equipment with a capacity that services need
- A pure, heavily tested availability engine: DST-correct working hours, buffers, notice and advance windows, a slot grid, class seats, resource capacity and a fair "any staff" choice (least booked that day)
- Double booking impossible under concurrency: ordered row locks around every decision plus a database exclusion constraint on staff time
- Waitlists for full classes, filled automatically when a seat frees up (cancellation, reschedule, larger class)
- A public booking page API: services, free slots in the guest's zone, booking with validation, honeypot and throttling, self-service by token (view, reschedule, cancel within windows), `.ics` downloads
- Every notice and reminder sent exactly once from maintenance — email with calendar invitations through the Mail app when installed, otherwise through the server's mailer (with an add-to-calendar link); in-app for staff, and copyable guest links when no mail can be sent
- Paid bookings become draft Sales Invoices at the booked price, exactly once; private HMAC-signed calendar feeds per staff member
- Reports: bookings summary, no-shows and cancellations, staff utilization (booked against available minutes)
- `packages/bookings/README.md` documents every entity, action, endpoint, setting and background job

## Install

Workspace administrators install it from **Apps** in Ferrum; it ships in the official Ferrum
server image. Self-hosters building their own image enable the `bookings` feature of the server.
Everything it adds is also available to AI agents over Ferrum's MCP server.

## Develop

Clone it next to the platform and point the platform dependencies at your checkout (this file
is git-ignored):

```toml
# .cargo/config.toml
[patch."https://github.com/CBFerrum/Ferrum"]
ferrum-kernel = { path = "../ferrum/crates/kernel" }
ferrum-meta = { path = "../ferrum/crates/meta" }
ferrum-domain = { path = "../ferrum/crates/domain" }
ferrum-calc = { path = "../ferrum/crates/calc" }
```

The tests need PostgreSQL with an ordinary (non-superuser) role — row-level security must apply:

```bash
createuser ferrum --pwprompt && createdb -O ferrum ferrum_bookings_test
```

```bash
cargo test
```

`FERRUM_BOOKINGS_TEST_DATABASE_URL` overrides the database URL. The app's metadata lives in `packages/` and is embedded
at build time; `app.yaml` is its manifest (see Ferrum's `docs/plan/apps.md`).

## License

AGPL-3.0-only, like the platform.

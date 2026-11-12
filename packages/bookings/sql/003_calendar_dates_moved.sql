-- Booking Settings' own dated openings and closures (calendar_dates) are superseded by the exceptions of
-- the platform's working calendars. The upkeep moves them once per workspace and records it here.
create table bookings_calendar_moved (
  tenant_id uuid primary key,
  moved_at timestamptz not null default now()
);
alter table bookings_calendar_moved enable row level security;
alter table bookings_calendar_moved force row level security;
create policy tenant_isolation on bookings_calendar_moved
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));

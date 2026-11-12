-- The upkeep now moves Booking Settings' old dated rows every time it finds new ones (not once per
-- workspace), so it remembers which rows it has dealt with.
create table bookings_calendar_rows_moved (
  tenant_id uuid not null,
  row_id uuid not null,
  moved_at timestamptz not null default now(),
  primary key (tenant_id, row_id)
);
alter table bookings_calendar_rows_moved enable row level security;
alter table bookings_calendar_rows_moved force row level security;
create policy tenant_isolation on bookings_calendar_rows_moved
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));

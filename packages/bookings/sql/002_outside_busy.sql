-- When staff members are busy in their outside calendars (read by their iCalendar addresses):
-- only the times, never what the events are. Each reading replaces the staff member's rows.
create table bookings_outside_busy (
  tenant_id uuid not null,
  staff uuid not null,
  from_time timestamptz not null,
  to_time timestamptz not null check (to_time > from_time),
  foreign key (tenant_id, staff) references e_booking_staff (tenant_id, id) on delete cascade
);
create index bookings_outside_busy_staff on bookings_outside_busy (tenant_id, staff, from_time);
alter table bookings_outside_busy enable row level security;
alter table bookings_outside_busy force row level security;
create policy tenant_isolation on bookings_outside_busy
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));

-- Bookings: the database's own guard against double booking, guest self-service tokens (only
-- their SHA-256 is kept) and the claim rows that make every notification go out exactly once.

-- A staff member's active bookings may not overlap, each counted with its own buffers — except
-- the seats of one class session, which share a session key (one-to-one bookings use their id).
alter table e_booking add constraint bookings_no_double_booking exclude using gist (
  tenant_id with =,
  staff with =,
  tstzrange(block_start, block_end, '[)') with &&,
  session_key with <>
) where (status in ('Pending', 'Confirmed'));

-- An unbounded range would overlap everything: active bookings always carry their whole schedule.
alter table e_booking add constraint bookings_active_is_scheduled check (
  status not in ('Pending', 'Confirmed')
  or (staff is not null and session_key is not null and block_start is not null and block_end is not null and block_end > block_start)
);

create index bookings_unnotified on e_booking (tenant_id, updated_at) where notice_version > notified_version;
create index bookings_confirmed_start on e_booking (tenant_id, start_time) where status = 'Confirmed';
create index bookings_waitlist on e_booking (tenant_id, session_key, created_at) where status = 'Waitlisted';
create index bookings_resource_time on e_booking (tenant_id, resource, block_start) where status in ('Pending', 'Confirmed') and resource is not null;

-- Guest links: `/manage/{token}` grants access to one booking until 30 days after it ends.
create table bookings_token (
  tenant_id uuid not null,
  token_hash text not null check (length(token_hash) = 64),
  booking_id uuid not null,
  created_at timestamptz not null default now(),
  primary key (tenant_id, token_hash),
  foreign key (tenant_id, booking_id) references e_booking (tenant_id, id) on delete cascade
);
create index bookings_token_booking on bookings_token (tenant_id, booking_id);
alter table bookings_token enable row level security;
alter table bookings_token force row level security;
create policy tenant_isolation on bookings_token
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));

-- One row per notification sent (or deliberately skipped), inserted in the transaction that sends it.
create table bookings_notice (
  tenant_id uuid not null,
  booking_id uuid not null,
  kind text not null check (kind in ('change', 'reminder')),
  key text not null check (length(key) between 1 and 100),
  channel text not null,
  created_at timestamptz not null default now(),
  primary key (tenant_id, booking_id, kind, key),
  foreign key (tenant_id, booking_id) references e_booking (tenant_id, id) on delete cascade
);
alter table bookings_notice enable row level security;
alter table bookings_notice force row level security;
create policy tenant_isolation on bookings_notice
  using (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()))
  with check (tenant_id = (select ferrum_tenant()) or (select ferrum_is_system()));

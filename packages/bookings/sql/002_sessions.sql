-- Class sessions scheduled before anyone books them (`BookingSession`). A booking and a session cannot
-- share one constraint (two tables): both lock the teacher's `e_booking_staff` row first and decide from
-- a calendar read after the lock; these guard what one table can.

-- A staff member's scheduled sessions may not overlap one another, each with its buffers.
alter table e_booking_session add constraint bookings_sessions_do_not_overlap exclude using gist (
  tenant_id with =,
  staff with =,
  tstzrange(block_start, block_end, '[)') with &&
) where (status = 'Scheduled');

alter table e_booking_session add constraint bookings_session_is_scheduled check (
  status <> 'Scheduled'
  or (staff is not null and session_key is not null and block_start is not null and block_end is not null and block_end > block_start)
);

-- A key names one session.
create unique index bookings_session_key on e_booking_session (tenant_id, session_key);
create index bookings_sessions_ahead on e_booking_session (tenant_id, staff, block_start) where status = 'Scheduled';

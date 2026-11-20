-- Class sessions scheduled before anyone books them (`BookingSession`). A booking and a session cannot
-- share one constraint (two tables): both lock the teacher's `e_booking_staff` row first and decide from
-- a calendar read after the lock; these guard what one table can. The session's unique key and its
-- index by teacher and time are the entity's own (`unique`, `indexes`), built by the schema sync without
-- holding writes. Each statement may run again.

-- A staff member's scheduled sessions may not overlap one another, each with its buffers.
alter table e_booking_session drop constraint if exists bookings_sessions_do_not_overlap;
alter table e_booking_session add constraint bookings_sessions_do_not_overlap exclude using gist (
  -- window: safe, PostgreSQL cannot add an exclusion constraint `not valid` (nor from an index), so it is added whole.
  -- The table is empty when it runs: this file ships with the entity (bookings 0.14.0, the first version with
  -- sessions), and the schema sync makes the table and runs this file in one start, before any request. No workspace
  -- is upgraded from a version that has sessions without it: one that ran an earlier text of this file is refused at
  -- start by the kernel's checksum (a package's SQL file is never re-run changed). Were rows there all the same, the
  -- add checks every one and refuses an overlap, so the guarantee never lapses unchecked.
  tenant_id with =,
  staff with =,
  tstzrange(block_start, block_end, '[)') with &&
) where (status = 'Scheduled');

alter table e_booking_session drop constraint if exists bookings_session_is_scheduled;
alter table e_booking_session add constraint bookings_session_is_scheduled check (
  status <> 'Scheduled'
  or (staff is not null and session_key is not null and block_start is not null and block_end is not null and block_end > block_start)
) not valid;
alter table e_booking_session validate constraint bookings_session_is_scheduled;

// The team's own screens, at the top of each.

import { NavLink } from "react-router";

export function Tabs() {
  return (
    <nav className="bk-tabs" aria-label="Bookings">
      <NavLink to="/" end>
        Schedule
      </NavLink>
      <NavLink to="/today">Today</NavLink>
      <NavLink to="/page">Booking page</NavLink>
    </nav>
  );
}

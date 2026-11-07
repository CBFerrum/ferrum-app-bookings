// The team's own screens, at the top of each.

import { NavLink } from "react-router";
import { t } from "@ferrum/client";

export function Tabs() {
  return (
    <nav className="bk-tabs" aria-label={t("Bookings")}>
      <NavLink to="/" end>
        {t("Schedule")}
      </NavLink>
      <NavLink to="/today">{t("Today")}</NavLink>
      <NavLink to="/page">{t("Booking page")}</NavLink>
    </nav>
  );
}

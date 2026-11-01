import { describe, expect, it } from "vitest";
import { addDays, addMonths, clockText, dayBounds, dayOf, instantAt, isDate, monthEnd, monthWeeks, offset, weekStart, wall, whenText } from "./zone";

const iso = (ms: number) => new Date(ms).toISOString();

describe("a zone's wall clock", () => {
  it("reads the hour a zone keeps at an instant", () => {
    const t = Date.parse("2026-11-02T04:30:00Z");
    expect(wall(t, "Asia/Kolkata")).toMatchObject({ y: 2026, m: 11, d: 2, h: 10, mi: 0 });
    expect(wall(t, "America/Los_Angeles")).toMatchObject({ d: 1, h: 20, mi: 30 });
    expect(offset(t, "Asia/Kolkata")).toBe(5.5 * 3_600_000);
  });

  it("names midnight 0, never 24", () => {
    expect(wall(Date.parse("2026-11-02T00:00:00Z"), "UTC").h).toBe(0);
  });

  it("finds the instant a wall clock names", () => {
    expect(iso(instantAt("Asia/Kolkata", "2026-11-02", 10 * 60))).toBe("2026-11-02T04:30:00.000Z");
    expect(iso(instantAt("America/New_York", "2026-07-01", 9 * 60))).toBe("2026-07-01T13:00:00.000Z");
    expect(iso(instantAt("UTC", "2026-11-02", 1440))).toBe("2026-11-03T00:00:00.000Z");
  });

  it("takes the first moment after a gap the clocks skip, and the first pass of a repeated time", () => {
    // London, 28 March 2027: 01:00 jumps to 02:00.
    expect(iso(instantAt("Europe/London", "2027-03-28", 90))).toBe("2027-03-28T01:00:00.000Z");
    expect(iso(instantAt("Europe/London", "2027-03-28", 180))).toBe("2027-03-28T02:00:00.000Z");
    // 31 October 2027: 02:00 falls back to 01:00, so 01:30 happens twice.
    expect(iso(instantAt("Europe/London", "2027-10-31", 90))).toBe("2027-10-31T00:30:00.000Z");
    expect(iso(instantAt("Europe/London", "2027-10-31", 150))).toBe("2027-10-31T02:30:00.000Z");
  });
});

describe("a day", () => {
  it("is 24 hours, except where the clocks change", () => {
    const hours = (zone: string, date: string) => {
      const [a, b] = dayBounds(zone, date);
      return (b - a) / 3_600_000;
    };
    expect(hours("Asia/Kolkata", "2026-11-02")).toBe(24);
    expect(hours("Europe/London", "2027-03-28")).toBe(23);
    expect(hours("Europe/London", "2027-10-31")).toBe(25);
  });

  it("begins at the zone's midnight", () => {
    expect(iso(dayBounds("Asia/Kolkata", "2026-11-02")[0])).toBe("2026-11-01T18:30:00.000Z");
    expect(dayOf(Date.parse("2026-11-01T18:30:00Z"), "Asia/Kolkata")).toBe("2026-11-02");
    expect(dayOf(Date.parse("2026-11-01T18:29:00Z"), "Asia/Kolkata")).toBe("2026-11-01");
  });
});

describe("dates", () => {
  it("does calendar arithmetic", () => {
    expect(addDays("2026-02-27", 2)).toBe("2026-03-01");
    expect(addDays("2026-12-31", 1)).toBe("2027-01-01");
    expect(addDays("2026-03-01", -1)).toBe("2026-02-28");
    expect(addMonths("2026-11", 2)).toBe("2027-01");
    expect(addMonths("2026-01", -1)).toBe("2025-12");
    expect(monthEnd("2028-02")).toBe("2028-02-29");
  });

  it("starts weeks where the reader does", () => {
    expect(weekStart("2026-11-04", 1)).toBe("2026-11-02"); // a Wednesday; weeks from Monday
    expect(weekStart("2026-11-04", 0)).toBe("2026-11-01"); // weeks from Sunday
    expect(weekStart("2026-11-02", 1)).toBe("2026-11-02");
    expect(weekStart("2026-11-01", 1)).toBe("2026-10-26");
  });

  it("lays a month out in whole weeks", () => {
    const weeks = monthWeeks("2026-11", 1);
    // 1 November 2026 is a Sunday and the 30th a Monday: six weeks.
    expect(weeks).toHaveLength(6);
    expect(weeks[0][0]).toBe("2026-10-26");
    expect(weeks[0][6]).toBe("2026-11-01");
    expect(weeks[5][0]).toBe("2026-11-30");
    expect(weeks[5][6]).toBe("2026-12-06");
    expect(weeks.every((w) => w.length === 7)).toBe(true);
    // February 2027 starts on a Monday and fits four weeks.
    expect(monthWeeks("2027-02", 1)).toHaveLength(4);
    expect(monthWeeks("2026-11", 0)[0][0]).toBe("2026-11-01");
  });

  it("accepts only real dates", () => {
    expect(isDate("2026-11-02")).toBe(true);
    expect(isDate("2026-02-30")).toBe(false);
    expect(isDate("02/11/2026")).toBe(false);
    expect(isDate(null)).toBe(false);
  });
});

describe("clock text", () => {
  const t = Date.parse("2026-11-02T04:30:00Z");
  it("keeps 12 or 24 hour time", () => {
    expect(clockText(t, "Asia/Kolkata", false)).toBe("10:00");
    expect(clockText(t, "Asia/Kolkata", true).replace(/\s/g, " ")).toMatch(/^10:00 AM$/i);
    expect(clockText(Date.parse("2026-11-02T00:05:00Z"), "UTC", false)).toBe("00:05");
  });

  it("writes a whole date and time", () => {
    expect(whenText(t, "Asia/Kolkata", false)).toContain("10:00");
    expect(whenText(t, "Asia/Kolkata", false)).toContain("2026");
  });
});

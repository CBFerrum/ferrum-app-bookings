import { describe, expect, it } from "vitest";
import { ApiError } from "@ferrum/client";
import { bookBody, calendarLinks, check, depositDue, durationText, firstDay, hasErrors, isLink, isTaken, onlyPerson, openDays, payLink, pollEvery, priceText, serverErrors, whereText, type GuestBooking, type Question, type Service, type Slots } from "./booking";

const questions: Question[] = [
  { question: "Any allergies?", type: "Yes/No", options: [], required: true },
  { question: "Age", type: "Number", options: [], required: false },
  { question: "Pressure", type: "Choice", options: ["Soft", "Firm"], required: false },
];

const service = (over: Partial<Service> = {}): Service => ({
  id: "svc-1",
  name: "Massage",
  duration_minutes: 60,
  seats: 1,
  price: "1500.00",
  location_type: "In Person",
  staff: [{ id: "st-1", name: "Asha" }],
  questions,
  ...over,
});

describe("the days that have times", () => {
  const slots: Slots = {
    time_zone: "Asia/Kolkata",
    from: "2026-11-02",
    to: "2026-11-08",
    days: [
      { date: "2026-11-05", slots: [{ start: "2026-11-05T04:30:00.000Z", end: "2026-11-05T05:30:00.000Z", time: "10:00", seats_left: 1, staff: [{ id: "st-1", name: "Asha" }] }] },
      { date: "2026-11-03", slots: [{ start: "2026-11-03T04:30:00.000Z", end: "2026-11-03T05:30:00.000Z", time: "10:00", seats_left: 1, staff: [{ id: "st-1", name: "Asha" }, { id: "st-2", name: "Ravi" }] }] },
      { date: "2026-11-04", slots: [] },
    ],
  };

  it("keeps the days with times and finds the first", () => {
    const open = openDays(slots);
    expect([...open.keys()].sort()).toEqual(["2026-11-03", "2026-11-05"]);
    expect(firstDay(open)).toBe("2026-11-03");
    expect(firstDay(openDays(undefined))).toBeNull();
  });

  it("books a named person only when the time names one", () => {
    expect(onlyPerson(slots.days[0].slots[0])).toBe("st-1");
    expect(onlyPerson(slots.days[1].slots[0])).toBeUndefined();
  });
});

describe("how a service reads", () => {
  it("writes lengths of time in words", () => {
    expect([30, 60, 90, 120].map(durationText)).toEqual(["30 min", "1 hour", "1 h 30 min", "2 hours"]);
  });

  it("says free for no price, and uses the workspace's currency otherwise", () => {
    expect(priceText("0.00", "INR")).toBe("Free");
    expect(priceText(null, null)).toBe("Free");
    expect(priceText("1500.00", "INR")).toMatch(/1,500/);
    expect(priceText("1500", null)).toMatch(/1,500/);
    expect(priceText("20", "not-a-currency")).toContain("20");
  });

  it("says how it meets, and shows a link as a link", () => {
    expect(whereText("In Person", "12 High St")).toBe("In person · 12 High St");
    expect(whereText("In Person")).toBe("In person");
    expect(whereText("Video")).toBe("Video call");
    expect(isLink("https://meet.example.com/abc")).toBe(true);
    expect(isLink("javascript:alert(1)")).toBe(false);
    expect(isLink("12 High St")).toBe(false);
  });
});

describe("the form", () => {
  const form = { name: "Jane", email: "jane@example.com", phone: "", notes: "", seats: 1, answers: { "Any allergies?": "No" } };

  it("is checked before it is sent", () => {
    expect(hasErrors(check(form, questions))).toBe(false);
    const bad = check({ ...form, name: " ", email: "jane@", answers: { Age: "old" } }, questions);
    expect(bad.name).toBeTruthy();
    expect(bad.email).toMatch(/email address/);
    expect(bad.answers).toEqual({ "Any allergies?": "Please answer this", Age: "Enter a number" });
    expect(check({ ...form, email: "" }, questions).email).toMatch(/confirmation/);
  });

  it("builds the request the site takes", () => {
    const body = bookBody({ service: service(), start: "2026-11-03T04:30:00.000Z", form: { ...form, phone: " 98 ", notes: "  ", answers: { "Any allergies?": "No", Age: " " } }, zone: "Asia/Kolkata" });
    expect(body).toMatchObject({ service: "svc-1", start_time: "2026-11-03T04:30:00.000Z", staff: "any", seats: 1, name: "Jane", phone: "98", notes: null, time_zone: "Asia/Kolkata", website: "" });
    expect(body.answers).toEqual([{ question: "Any allergies?", answer: "No" }]);
    const named = bookBody({ service: service({ seats: 8 }), start: "x", staff: "st-2", form: { ...form, seats: 3 }, zone: "UTC", waitlist: true });
    expect([named.staff, named.seats, named.waitlist]).toEqual(["st-2", 3, true]);
    expect(bookBody({ service: service(), start: "x", form: { ...form, seats: 5 }, zone: "UTC" }).seats).toBe(1);
  });

  it("carries the language the page is shown in", () => {
    expect(bookBody({ service: service(), start: "x", form, zone: "UTC", language: "hi" }).language).toBe("hi");
  });

  it("asks for a series only where the service offers one, never for a waitlist, and no longer than it allows", () => {
    const weekly = { ...form, every: "week", times: 5 };
    expect(bookBody({ service: service({ repeats: 3 }), start: "x", form: weekly, zone: "UTC" })).toMatchObject({ repeat_every: "week", repeat_count: 3 });
    expect(bookBody({ service: service({ repeats: 3 }), start: "x", form: { ...weekly, times: 1 }, zone: "UTC" })).toMatchObject({ repeat_count: 2 });
    expect(bookBody({ service: service(), start: "x", form: weekly, zone: "UTC" })).not.toHaveProperty("repeat_every");
    expect(bookBody({ service: service({ repeats: 3, seats: 8 }), start: "x", form: weekly, zone: "UTC", waitlist: true })).not.toHaveProperty("repeat_every");
    expect(bookBody({ service: service({ repeats: 3 }), start: "x", form: { ...weekly, every: "" }, zone: "UTC" })).not.toHaveProperty("repeat_count");
  });
});

describe("the server's complaints", () => {
  const fail = (...fields: [string, string][]) => new ApiError(422, "invalid", "Please check your input.", fields.map(([field, message]) => ({ field, message })));

  it("sit beside their fields", () => {
    const e = serverErrors(fail(["email", "is required"], ["name", "at most 200 characters"], ["answers", "Age: must be a number"], ["answers", "Pressure: choose one of the options"]), questions);
    expect(e.email).toBe("Email is required");
    expect(e.name).toBe("Your name can have at most 200 characters");
    expect(e.answers).toEqual({ Age: "must be a number", Pressure: "choose one of the options" });
    expect(e.general).toEqual([]);
  });

  it("say the rest in a sentence of their own", () => {
    const e = serverErrors(fail(["start_time", "Asha is not available at that time (already booked then)"], ["time_zone", "is not an IANA time zone"]), questions);
    expect(e.general).toEqual(["Asha is not available at that time (already booked then)", "time zone: is not an IANA time zone"]);
    expect(serverErrors(new ApiError(429, "too_many_requests", "Slow down."), questions).general).toEqual(["Slow down."]);
    expect(serverErrors(new Error("boom"), questions).general).toEqual(["boom"]);
  });

  it("know when the time was taken", () => {
    expect(isTaken(new ApiError(409, "conflict", "no staff member is available at that time"))).toBe(true);
    expect(isTaken(fail(["start_time", "outside working hours"]))).toBe(true);
    expect(isTaken(fail(["email", "is required"]))).toBe(false);
    expect(isTaken(new Error("x"))).toBe(false);
  });
});

describe("the guest's calendar", () => {
  const start = Date.parse("2026-11-03T04:30:00Z");
  const links = calendarLinks({ title: "Massage with Asha", start, end: start + 3_600_000, where: "12 High St & Co", details: "Booking BKG-1" });

  it("links to Google Calendar with the time in UTC", () => {
    expect(links.google).toContain("dates=20261103T043000Z%2F20261103T053000Z");
    expect(links.google).toContain("text=Massage%20with%20Asha");
    expect(links.google).toContain("location=12%20High%20St%20%26%20Co");
  });

  it("links to Outlook on the web", () => {
    expect(links.outlook).toContain("startdt=2026-11-03T04%3A30%3A00.000Z");
    expect(links.outlook).toContain("subject=Massage%20with%20Asha");
  });

  it("leaves out what it does not have", () => {
    expect(calendarLinks({ title: "x", start, end: start + 1000 }).google).not.toContain("location");
  });
});

describe("a deposit that is due", () => {
  const deposit = (status: string, url: string | null = "https://pay.test/s/1") => ({ amount: "300.00", currency: "INR", status, payment_url: url, kept: null, pay_by: null });
  const b = (status: string, d: ReturnType<typeof deposit> | null | undefined) => ({ status, deposit: d }) as unknown as GuestBooking;

  it("is due while the request waits and the booking is held or expired", () => {
    expect(depositDue(b("Pending", deposit("Initiated")))).toBe(true);
    expect(depositDue(b("Expired", deposit("Initiated")))).toBe(true);
    expect(depositDue(b("Pending", deposit("Paid")))).toBe(false);
    expect(depositDue(b("Confirmed", deposit("Initiated")))).toBe(false);
    expect(depositDue(b("Pending", null))).toBe(false);
    expect(depositDue(b("Pending", undefined))).toBe(false);
  });

  it("links only to an https address", () => {
    expect(payLink(b("Pending", deposit("Initiated")))).toBe("https://pay.test/s/1");
    for (const bad of ["http://x.test", "javascript:alert(1)", "https://x.test/a b", "", null]) expect(payLink(b("Pending", deposit("Initiated", bad)))).toBeNull();
    expect(payLink(b("Pending", deposit("Paid")))).toBeNull();
  });

  it("asks again until the payment page exists, then until the payment arrives, then stops", () => {
    const wait = b("Pending", deposit("Initiated", null));
    expect(pollEvery(wait, 0)).toBe(3000);
    expect(pollEvery(wait, 59_999)).toBe(3000);
    expect(pollEvery(wait, 60_000)).toBe(15_000);
    expect(pollEvery(b("Pending", deposit("Initiated")), 0)).toBe(15_000);
    expect(pollEvery(b("Confirmed", deposit("Paid")), 0)).toBe(false);
    expect(pollEvery(undefined, 0)).toBe(false);
  });
});

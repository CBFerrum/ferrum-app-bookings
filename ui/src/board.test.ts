import { describe, expect, it } from "vitest";
import { type Bk, blocks, clashed, clip, dropHint, hhmm, hourMarks, initials, inOrder, landing, makeLane, mergeSpans, nextSteps, pack, type ServiceRow, snap, tone, viewWindow, workSpans } from "./board";
import { instantAt } from "./zone";

const Z = "Asia/Kolkata";
const at = (date: string, hhmm: string, zone = Z) => instantAt(zone, date, Number(hhmm.slice(0, 2)) * 60 + Number(hhmm.slice(3)));
const iso = (ms: number) => new Date(ms).toISOString();

const services = new Map<string, ServiceRow>([
  ["massage", { id: "massage", service_name: "Massage", capacity: 1 }],
  ["pilates", { id: "pilates", service_name: "Pilates", capacity: 8 }],
]);

function booking(id: string, over: Partial<Bk> & { from?: string; to?: string; day?: string } = {}): Bk {
  const day = over.day ?? "2026-11-02";
  return {
    id,
    service: "massage",
    service_name: "Massage",
    staff: "asha",
    start_time: iso(at(day, over.from ?? "10:00")),
    end_time: iso(at(day, over.to ?? "11:00")),
    seats: 1,
    status: "Confirmed",
    guest_name: `Guest ${id}`,
    session_key: id,
    ...over,
  };
}

describe("hours and spans", () => {
  it("reads HH:MM", () => {
    expect(hhmm("09:30")).toBe(570);
    expect(hhmm("24:00")).toBe(1440);
    expect([hhmm("24:01"), hhmm("9:30"), hhmm("12:60"), hhmm(null)]).toEqual([null, null, null, null]);
  });

  it("merges touching spans and clips to a day", () => {
    expect(mergeSpans([[5, 8], [1, 3], [3, 4], [7, 9]])).toEqual([[1, 4], [5, 9]]);
    expect(mergeSpans([[2, 2]])).toEqual([]);
    expect(clip([[0, 10], [12, 20]], [5, 15])).toEqual([[5, 10], [12, 15]]);
  });

  it("puts a staff member's weekly hours on the day, in their own zone", () => {
    const hours = [
      { weekday: "Monday", start_time: "09:00", end_time: "12:00" },
      { weekday: "Monday", start_time: "13:00", end_time: "17:00" },
      { weekday: "Tuesday", start_time: "09:00", end_time: "17:00" },
    ];
    const [from, to] = [at("2026-11-02", "00:00"), at("2026-11-03", "00:00")];
    const spans = workSpans(Z, hours, from, to);
    expect(spans.map(([a, b]) => [iso(a), iso(b)])).toEqual([
      ["2026-11-02T03:30:00.000Z", "2026-11-02T06:30:00.000Z"],
      ["2026-11-02T07:30:00.000Z", "2026-11-02T11:30:00.000Z"],
    ]);
    // 09:00 in Berlin is 13:30 in Kolkata: the shift sits later on a board kept in the workspace's zone.
    const berlin = workSpans("Europe/Berlin", hours, from, to);
    expect(iso(berlin[0][0])).toBe("2026-11-02T08:00:00.000Z");
  });

  it("keeps a shift that crosses midnight in the staff member's zone whole", () => {
    const night = [{ weekday: "Monday", start_time: "22:00", end_time: "24:00" }, { weekday: "Tuesday", start_time: "00:00", end_time: "06:00" }];
    const spans = workSpans("UTC", night, at("2026-11-03", "00:00", "UTC"), at("2026-11-04", "00:00", "UTC"));
    expect(spans.map(([a, b]) => [iso(a), iso(b)])).toEqual([["2026-11-03T00:00:00.000Z", "2026-11-03T06:00:00.000Z"]]);
  });
});

describe("blocks", () => {
  it("draws a booking as one block, with its buffers", () => {
    const b = booking("1", { block_start: iso(at("2026-11-02", "09:45")), block_end: iso(at("2026-11-02", "11:15")) });
    const [blk] = blocks([b], services);
    expect(blk).toMatchObject({ kind: "booking", title: "Guest 1", seats: 1, status: "Confirmed" });
    expect([blk.from, blk.start, blk.end, blk.to].map(iso)).toEqual(["2026-11-02T04:15:00.000Z", "2026-11-02T04:30:00.000Z", "2026-11-02T05:30:00.000Z", "2026-11-02T05:45:00.000Z"]);
  });

  it("draws the seats of a class as one session", () => {
    const key = "pilates/asha/1";
    const list = [
      booking("a", { service: "pilates", service_name: "Pilates", session_key: key, seats: 2 }),
      booking("b", { service: "pilates", service_name: "Pilates", session_key: key, seats: 1, status: "Pending" }),
      booking("c", { service: "pilates", service_name: "Pilates", session_key: key, status: "Cancelled" }),
    ];
    const out = blocks(list, services);
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({ kind: "session", title: "Pilates", seats: 3, capacity: 8, status: "Confirmed" });
    expect(out[0].bookings.map((b) => b.id)).toEqual(["a", "b"]);
  });

  it("leaves out what holds no time, and a class with one guest is still a class", () => {
    expect(blocks([booking("1", { status: "Cancelled" }), booking("2", { status: "Waitlisted" })], services)).toEqual([]);
    expect(blocks([booking("e", { status: "Expired" })], services)).toEqual([]);
    expect(blocks([booking("p", { service: "pilates", session_key: "k" })], services)[0].kind).toBe("session");
  });

  it("lists them by when they start", () => {
    const out = blocks([booking("2", { from: "12:00", to: "13:00" }), booking("1", { from: "09:00", to: "10:00" })], services);
    expect(out.map((b) => b.key)).toEqual(["1", "2"]);
  });
});

describe("overlapping blocks", () => {
  const item = (start: number, end: number, name = "") => ({ start, end, name });

  it("stand side by side, sharing the width of their cluster", () => {
    const p = pack([item(0, 60, "a"), item(30, 90, "b"), item(120, 180, "c")]);
    const by = Object.fromEntries(p.map((x) => [x.item.name, [x.col, x.cols]]));
    expect(by).toEqual({ a: [0, 2], b: [1, 2], c: [0, 1] });
  });

  it("reuses a column that has freed up", () => {
    const p = pack([item(0, 100, "a"), item(10, 50, "b"), item(60, 90, "c")]);
    const by = Object.fromEntries(p.map((x) => [x.item.name, [x.col, x.cols]]));
    expect(by).toEqual({ a: [0, 2], b: [1, 2], c: [1, 2] });
  });

  it("counts a block as long as it is drawn", () => {
    // A 5-minute booking is drawn at least 20 minutes tall, so it reaches the one starting at minute 10.
    expect(pack([item(0, 5, "short"), item(10, 60, "next")], 5).map((x) => x.cols)).toEqual([1, 1]);
    expect(pack([item(0, 5, "short"), item(10, 60, "next")], 20).map((x) => x.cols)).toEqual([2, 2]);
  });

  it("lets blocks that only touch share a column", () => {
    expect(pack([item(0, 60), item(60, 120)]).map((x) => [x.col, x.cols])).toEqual([[0, 1], [0, 1]]);
  });
});

describe("a day on the board", () => {
  const asha = { id: "asha", display_name: "Asha", email: "asha@x.test", time_zone: Z, hours: [{ weekday: "Monday", start_time: "09:00", end_time: "17:00" }] };

  it("shows working hours, time off and the day's own length", () => {
    const lane = makeLane({
      id: "asha",
      label: "Asha",
      date: "2026-11-02",
      zone: Z,
      people: [asha],
      offs: [{ id: "o", staff: "asha", from_time: iso(at("2026-11-02", "12:00")), to_time: iso(at("2026-11-02", "13:00")), reason: "Lunch" }],
    });
    expect((lane.end - lane.start) / 60_000).toBe(1440);
    expect(lane.work).toHaveLength(1);
    expect(lane.off).toEqual([[at("2026-11-02", "12:00"), at("2026-11-02", "13:00")]]);
    // Time off belongs to a person: a lane for several shows none.
    expect(makeLane({ id: "x", label: "All", date: "2026-11-02", zone: Z, people: [asha, { ...asha, id: "ravi" }], offs: [{ id: "o", staff: "asha", from_time: iso(at("2026-11-02", "12:00")), to_time: iso(at("2026-11-02", "13:00")) }] }).off).toEqual([]);
  });

  it("is 23 hours long when the clocks go forward", () => {
    const lane = makeLane({ id: "l", label: "London", date: "2027-03-28", zone: "Europe/London", people: [], offs: [] });
    expect((lane.end - lane.start) / 60_000).toBe(1380);
  });

  it("shows the hours people work and are booked, never fewer than eight", () => {
    const lane = makeLane({ id: "asha", label: "Asha", date: "2026-11-02", zone: Z, people: [asha], offs: [] });
    expect(viewWindow([lane], [])).toEqual({ from: 540, to: 1020 }); // Asha works 9 to 5
    const [early] = blocks([booking("1", { from: "07:15", to: "08:00" })], services);
    expect(viewWindow([lane], [early])).toEqual({ from: 420, to: 1020 });
    const bare = makeLane({ id: "b", label: "B", date: "2026-11-02", zone: Z, people: [], offs: [] });
    expect(viewWindow([bare], [])).toEqual({ from: 480, to: 1080 });
    const [late] = blocks([booking("2", { from: "21:00", to: "22:30" })], services);
    const w = viewWindow([bare], [late]);
    expect(w.to - w.from).toBeGreaterThanOrEqual(480);
    expect(w.to).toBeLessThanOrEqual(1440);
  });

  it("labels the hours as the wall clock reads", () => {
    const lane = makeLane({ id: "l", label: "L", date: "2026-11-02", zone: Z, people: [], offs: [] });
    const marks = hourMarks(lane, { from: 540, to: 780 }, Z, false);
    expect(marks.map((m) => m.label)).toEqual(["09:00", "10:00", "11:00", "12:00"]);
    expect(hourMarks(lane, { from: 540, to: 600 }, Z, true)[0].label.replace(/\s/g, " ")).toMatch(/^9:00 AM$/i);
  });
});

describe("dragging", () => {
  const lane = makeLane({ id: "asha", label: "Asha", date: "2026-11-02", zone: Z, people: [], offs: [] });

  it("lands on the quarter hour", () => {
    expect([0, 6, 8, 14, 22, 23].map(snap)).toEqual([0, 0, 15, 15, 15, 30]);
    // Window from 08:00; the block held 10 minutes below its top; let go 130 minutes down.
    expect(iso(landing(lane, { from: 480, to: 1080 }, 130, 10))).toBe(iso(at("2026-11-02", "10:00")));
  });

  it("warns of what a drop is likely to meet", () => {
    const [moving, other] = blocks(
      [
        booking("m", { from: "10:00", to: "11:00", block_start: iso(at("2026-11-02", "09:50")), block_end: iso(at("2026-11-02", "11:10")) }),
        booking("o", { from: "13:00", to: "14:00", guest_name: "Ben" }),
      ],
      services,
    );
    const work: [number, number][] = [[at("2026-11-02", "09:00"), at("2026-11-02", "18:00")]];
    const off: [number, number][] = [[at("2026-11-02", "16:00"), at("2026-11-02", "17:00")]];
    const hint = (time: string) => dropHint(moving, at("2026-11-02", time), [moving, other], work, off, true);
    expect(hint("11:30")).toEqual({ kind: "ok" });
    expect(hint("12:00")).toEqual({ kind: "overlap", with: "Ben" }); // its buffer reaches 13:10
    expect(hint("14:00")).toEqual({ kind: "overlap", with: "Ben" }); // and starts 10 minutes before it
    expect(hint("14:10")).toEqual({ kind: "ok" });
    expect(hint("15:30")).toEqual({ kind: "off" });
    expect(hint("17:30")).toEqual({ kind: "hours" });
    expect(dropHint(moving, at("2026-11-02", "17:30"), [], work, off, false)).toEqual({ kind: "ok" });
  });

  it("does not count a finished booking as in the way", () => {
    const [moving, done] = blocks([booking("m"), booking("d", { from: "12:00", to: "13:00", status: "Completed" })], services);
    expect(dropHint(moving, at("2026-11-02", "12:00"), [moving, done], [], [], false)).toEqual({ kind: "ok" });
  });
});

describe("what can be done to a booking", () => {
  const now = Date.parse("2026-11-02T10:00:00Z");
  const hour = (h: number) => new Date(now + h * 3_600_000).toISOString();

  it("follows the server's rules", () => {
    expect(nextSteps({ status: "Pending", start_time: hour(2) }, now)).toEqual(["confirm", "reschedule", "cancel_booking"]);
    expect(nextSteps({ status: "Confirmed", start_time: hour(2) }, now)).toEqual(["reschedule", "cancel_booking"]);
    expect(nextSteps({ status: "Confirmed", start_time: hour(-1) }, now)).toEqual(["reschedule", "cancel_booking", "complete", "mark_no_show"]);
    expect(nextSteps({ status: "Pending", start_time: hour(-1) }, now)).toEqual(["confirm", "reschedule", "cancel_booking", "mark_no_show"]);
    expect(nextSteps({ status: "Waitlisted", start_time: hour(2) }, now)).toEqual(["cancel_booking"]);
    for (const status of ["Cancelled", "Completed", "No Show"] as const) expect(nextSteps({ status, start_time: hour(-1) }, now)).toEqual([]);
  });

  it("orders a day: under way, still to come, done", () => {
    const list = [
      { id: "c", start_time: hour(2), end_time: hour(3) },
      { id: "a", start_time: hour(-3), end_time: hour(-2) },
      { id: "b", start_time: hour(-1), end_time: hour(1) },
      { id: "d", start_time: hour(1), end_time: hour(2) },
    ];
    const day = inOrder(list, now);
    expect([day.now, day.next, day.done].map((l) => l.map((x) => x.id))).toEqual([["b"], ["d", "c"], ["a"]]);
  });

  it("makes initials", () => {
    expect([initials("Asha Rao"), initials("ravi"), initials(null), initials("Mary Jane Watson")]).toEqual(["AR", "R", "?", "MW"]);
  });
});

describe("statuses and the closed-calendar mark", () => {
  it("colours Expired like Cancelled, and offers nothing for it", () => {
    expect(tone("Expired")).toBe(tone("Cancelled"));
    expect(nextSteps({ status: "Expired", start_time: new Date(Date.now() + 3_600_000).toISOString() }, Date.now())).toEqual([]);
  });

  it("marks a block only while one of its bookings still holds a closed time", () => {
    const one = (over: Partial<Bk>) => blocks([booking("1", over)], services)[0];
    expect(clashed(one({ closed_clash: true }))).toBe(true);
    expect(clashed(one({}))).toBe(false);
    expect(clashed(one({ closed_clash: true, status: "No Show" }))).toBe(false);
  });
});

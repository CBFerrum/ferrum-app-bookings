// The schedule as a time grid: one column per person, room or day, bookings sized to how long they
// last, their buffers shaded, working hours lit and time off hatched, a line at the current time.
// A booking moves by dragging it (to another time, or another person's column) or from the keyboard:
// focus it, press M, move it with the arrow keys, Enter to drop, Escape to leave it. Both show where
// it would land, and in red when it would clash; the server answers for real, and the caller puts the
// booking back, with its reason, when it refuses. Touch screens open the booking, whose sheet moves it.

import { TriangleAlert } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { clockText } from "./zone";
import { type Block, clashed, type Drop, dropText, hourMarks, landing, type Lane, minutesIn, pack, STEP, type Window } from "./board";
import { t as tr, isolate } from "@ferrum/client";

/** Pixels an hour takes, and the shortest a block is drawn (minutes). */
const HOUR = 56;
const PX = HOUR / 60;
const LEAST = 30;

interface Dragging {
  key: string;
  lane: string;
  start: number;
  source: "pointer" | "keyboard";
  hint: Drop;
}

export interface BoardProps {
  lanes: Lane[];
  blocks: Block[];
  /** The lane a block belongs to. */
  laneOf: (b: Block) => string | undefined;
  win: Window;
  zone: string;
  hour12: boolean;
  now: number;
  selected: string | null;
  canMove: boolean;
  /** Whether a block may change lane (another person, another day) as well as time. */
  acrossLanes: boolean;
  canCreate: boolean;
  /** Blocks say who they are with (when the lanes are not people). */
  showWho: boolean;
  whoInitials: (staff: string | null) => string;
  hintFor: (b: Block, lane: Lane, start: number) => Drop;
  note: (lane: Lane) => string;
  onOpen: (b: Block) => void;
  onCreate: (lane: Lane, start: number) => void;
  onMove: (b: Block, lane: Lane, start: number) => void;
  onLane: (lane: Lane) => void;
}

const statusNote = (b: Block) => (b.status === "Pending" ? "Pending" : b.status === "Completed" ? "Done" : b.status === "No Show" ? tr("No show") : "");

export function Board(p: BoardProps) {
  const { lanes, win } = p;
  const height = (win.to - win.from) * PX;
  const bodies = useRef(new Map<string, HTMLDivElement>());
  const latest = useRef(p);
  latest.current = p;
  const scroller = useRef<HTMLDivElement>(null);
  const [pressed, setPressed] = useState<{ key: string; lane: string; x: number; y: number; grab: number } | null>(null);
  const [drag, setDrag] = useState<Dragging | null>(null);
  const [said, setSaid] = useState("");
  const suppress = useRef(false);
  const refocus = useRef<string | null>(null);

  const byLane = useMemo(() => {
    const map = new Map<string, Block[]>();
    for (const b of p.blocks) {
      const id = p.laneOf(b);
      if (id) map.set(id, [...(map.get(id) ?? []), b]);
    }
    return map;
  }, [p.blocks, p.laneOf]);

  // The first screenful: now when it is today, otherwise the top of the day.
  const day = lanes[0]?.start;
  useEffect(() => {
    const el = scroller.current;
    const today = lanes.find((l) => p.now >= l.start && p.now < l.end);
    if (el) el.scrollTop = today ? Math.max(0, (minutesIn(today, p.now) - win.from - 60) * PX) : 0;
    // Once per day shown, not at every tick of the clock.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [day, win.from]);

  // A block that moved lands in another column: its replacement takes the focus the old one had.
  useEffect(() => {
    if (!refocus.current) return;
    document.getElementById(`bk-${refocus.current}`)?.focus();
    refocus.current = null;
  });

  const commit = (d: Dragging) => {
    const { blocks, lanes: all, onMove } = latest.current;
    const block = blocks.find((b) => b.key === d.key);
    const lane = all.find((l) => l.id === d.lane);
    refocus.current = d.key;
    // The click that follows a drop (a pointer's let-go, a key's release) is not a press on the booking.
    suppress.current = true;
    setTimeout(() => (suppress.current = false), 250);
    setDrag(null);
    setPressed(null);
    if (block && lane && (d.start !== block.start || d.lane !== latest.current.laneOf(block))) onMove(block, lane, d.start);
  };

  // Dragging with a mouse or pen: the page listens from the press to the let-go.
  useEffect(() => {
    if (!pressed) return;
    let last: Dragging | null = null;
    const laneAt = (x: number) => {
      if (!latest.current.acrossLanes) return pressed.lane;
      for (const l of latest.current.lanes) {
        const r = bodies.current.get(l.id)?.getBoundingClientRect();
        if (r && x >= r.left && x < r.right) return l.id;
      }
      return pressed.lane;
    };
    const onMove = (e: PointerEvent) => {
      if (!last && Math.hypot(e.clientX - pressed.x, e.clientY - pressed.y) < 5) return;
      const { lanes: all, blocks, win: w, hintFor } = latest.current;
      const id = laneAt(e.clientX);
      const lane = all.find((l) => l.id === id);
      const body = bodies.current.get(id);
      const block = blocks.find((b) => b.key === pressed.key);
      if (!lane || !body || !block) return;
      // Never out of the day it is dropped on.
      const start = Math.min(Math.max(landing(lane, w, (e.clientY - body.getBoundingClientRect().top) / PX, pressed.grab), lane.start), lane.end - (block.end - block.start));
      last = { key: pressed.key, lane: id, start, source: "pointer", hint: hintFor(block, lane, start) };
      setDrag(last);
    };
    const onUp = () => {
      if (last) commit(last);
      else setPressed(null);
    };
    const leave = () => {
      setDrag(null);
      setPressed(null);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && leave();
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", leave);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", leave);
      window.removeEventListener("keydown", onKey);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pressed]);

  // Moving from the keyboard.
  useEffect(() => {
    if (drag?.source !== "keyboard") return;
    const onKey = (e: KeyboardEvent) => {
      const { lanes: all, blocks, acrossLanes, hintFor, zone, hour12 } = latest.current;
      const block = blocks.find((b) => b.key === drag.key);
      const at = all.findIndex((l) => l.id === drag.lane);
      if (!block || at < 0) return;
      let to = at;
      let start = drag.start;
      if (e.key === "ArrowUp") start -= STEP * 60_000;
      else if (e.key === "ArrowDown") start += STEP * 60_000;
      else if (e.key === "ArrowLeft" && acrossLanes) to = Math.max(0, at - 1);
      else if (e.key === "ArrowRight" && acrossLanes) to = Math.min(all.length - 1, at + 1);
      else if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        e.stopPropagation();
        return commit(drag);
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        refocus.current = drag.key;
        setSaid(tr("{title} stays where it was.", { title: block.title }));
        return setDrag(null);
      } else return;
      e.preventDefault();
      e.stopPropagation();
      const lane = all[to];
      // A day to the side keeps the time of day; the quarter hours stay inside the day.
      start += lane.start - all[at].start;
      start = Math.min(Math.max(start, lane.start), lane.end - (block.end - block.start));
      const hint = hintFor(block, lane, start);
      setDrag({ ...drag, lane: lane.id, start, hint });
      setSaid(tr("{title}: {start} to {value}, {label}. {hint} Enter to move it here, Escape to leave it.", { title: block.title, start: clockText(start, zone, hour12), value: clockText(start + (block.end - block.start), zone, hour12), label: lane.label, hint: dropText(hint) }));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drag]);

  const pickUp = (b: Block, lane: string) => {
    const d: Dragging = { key: b.key, lane, start: b.start, source: "keyboard", hint: { kind: "ok" } };
    setDrag(d);
    setSaid((p.acrossLanes ? tr("Moving {title}. Arrow keys move it between columns and by {step} minutes, Enter to move it here, Escape to leave it.", { title: b.title, step: STEP }) : tr("Moving {title}. Arrow keys move it by {step} minutes, Enter to move it here, Escape to leave it.", { title: b.title, step: STEP })));
  };

  const at = (t: number, lane: Lane) => (minutesIn(lane, t) - win.from) * PX;
  const range = (b: { start: number; end: number }) => `${clockText(b.start, p.zone, p.hour12)} – ${clockText(b.end, p.zone, p.hour12)}`;

  return (
    <div className="bk-board">
      <div className="bk-scroll" ref={scroller} tabIndex={-1}>
        <div className="bk-grid" style={{ gridTemplateColumns: `4.5rem repeat(${lanes.length}, minmax(9.5rem, 1fr))` }} data-dragging={drag?.source}>
          <div className="bk-corner" />
          {lanes.map((l) => (
            <button key={l.id} type="button" className="bk-lane-head" data-today={(p.now >= l.start && p.now < l.end) || undefined} onClick={() => p.onLane(l)}>
              <strong>{isolate(l.label)}</strong>
              {l.sub && <span>{isolate(l.sub)}</span>}
              <small>{isolate(p.note(l))}</small>
            </button>
          ))}
          <div className="bk-gutter" style={{ height }} aria-hidden="true">
            {lanes[0] &&
              hourMarks(lanes[0], win, p.zone, p.hour12).map((m) => (
                <span key={m.at} style={{ top: (m.at - win.from) * PX }}>
                  {isolate(m.label)}
                </span>
              ))}
          </div>
          {lanes.map((lane) => {
            const list = byLane.get(lane.id) ?? [];
            const placed = pack(list, LEAST * 60_000);
            const nowAt = p.now >= lane.start && p.now < lane.end ? at(p.now, lane) : null;
            return (
              <div
                key={lane.id}
                ref={(el) => void (el ? bodies.current.set(lane.id, el) : bodies.current.delete(lane.id))}
                className="bk-lane"
                style={{ height }}
                role="group"
                aria-label={`${lane.label}${lane.sub ? ` ${lane.sub}` : ""}`}
                data-create={p.canCreate || undefined}
                onClick={(e) => {
                  // A drag ends in a click wherever it was dropped: that is no request for a new booking.
                  if (!p.canCreate || suppress.current || (e.target as Element).closest(".bk-slot")) return;
                  // The quarter hour that was pressed.
                  const y = (e.clientY - e.currentTarget.getBoundingClientRect().top) / PX;
                  p.onCreate(lane, lane.start + (win.from + Math.floor(y / STEP) * STEP) * 60_000);
                }}
              >
                {lane.work.map(([a, b]) => (
                  <div key={a} className="bk-work" style={{ top: at(a, lane), height: ((b - a) / 60_000) * PX }} />
                ))}
                {lane.off.map(([a, b]) => (
                  <div key={a} className="bk-off" style={{ top: at(a, lane), height: ((b - a) / 60_000) * PX }}>
                    <span>{tr("Time off")}</span>
                  </div>
                ))}
                {nowAt !== null && nowAt >= 0 && nowAt <= height && <div className="bk-now" style={{ top: nowAt }} aria-hidden="true" />}
                {placed.map(({ item: b, col, cols }) => {
                  const lifted = drag?.key === b.key;
                  const before = Math.max(0, (b.start - b.from) / 60_000);
                  const after = Math.max(0, (b.to - b.end) / 60_000);
                  const minutes = (b.end - b.start) / 60_000;
                  const h = Math.max(minutes * PX, LEAST * PX * 0.7);
                  const first = b.bookings[0];
                  const label = `${range(b)}, ${b.title}${b.kind === "session" ? tr(", {seats} of {capacity} seats", { seats: b.seats, capacity: b.capacity }) : first.service_name ? `, ${first.service_name}` : ""}${clashed(b) ? tr(", outside opening hours") : ""}, ${b.status}`;
                  return (
                    <div key={b.key} className="bk-slot" style={{ top: at(b.from, lane), left: `calc(${(col / cols) * 100}% + 2px)`, width: `calc(${100 / cols}% - 4px)` }} data-lifted={lifted || undefined}>
                      {before > 0 && <div className="bk-buffer" style={{ height: before * PX }} aria-hidden="true" />}
                      <button
                        type="button"
                        id={`bk-${b.key}`}
                        className="bk-block"
                        data-status={b.status}
                        data-clash={clashed(b) || undefined}
                        data-kind={b.kind}
                        data-selected={p.selected === b.key || undefined}
                        data-short={h < 44 || undefined}
                        style={{ height: h }}
                        aria-label={label}
                        aria-keyshortcuts={p.canMove && b.kind === "booking" ? "M" : undefined}
                        title={`${label}${clashed(b) ? `. ${tr("A working calendar closed this time after it was booked. Move it, or confirm with the guest that it stands.")}` : ""}${p.canMove && b.kind === "booking" ? ". Drag to move it, or press M." : ""}`}
                        onPointerDown={(e) => {
                          if (!p.canMove || b.kind !== "booking" || e.button !== 0 || e.pointerType === "touch") return;
                          setPressed({ key: b.key, lane: lane.id, x: e.clientX, y: e.clientY, grab: (e.clientY - e.currentTarget.getBoundingClientRect().top) / PX });
                        }}
                        onClick={() => !suppress.current && p.onOpen(b)}
                        onKeyDown={(e) => {
                          if ((e.key === "m" || e.key === "M") && p.canMove && b.kind === "booking" && !e.metaKey && !e.ctrlKey && !e.altKey) {
                            e.preventDefault();
                            pickUp(b, lane.id);
                          }
                        }}
                      >
                        <span className="bk-block-time">
                          {clashed(b) && <TriangleAlert size={12} className="bk-block-warn" aria-hidden="true" />}
                          {h < 44 ? clockText(b.start, p.zone, p.hour12) : range(b)}
                        </span>
                        <strong className="bk-block-title">{isolate(b.title)}</strong>
                        <span className="bk-block-sub">
                          {b.kind === "session" ? tr("{seats} of {capacity} seats", { seats: b.seats, capacity: b.capacity }) : first.service_name}
                          {p.showWho && b.staff ? ` · ${p.whoInitials(b.staff)}` : ""}
                          {statusNote(b) ? ` · ${statusNote(b)}` : ""}
                        </span>
                      </button>
                      {after > 0 && <div className="bk-buffer" style={{ height: after * PX }} aria-hidden="true" />}
                    </div>
                  );
                })}
                {drag && drag.lane === lane.id && (() => {
                  const b = p.blocks.find((x) => x.key === drag.key);
                  if (!b) return null;
                  const end = drag.start + (b.end - b.start);
                  return (
                    <div className="bk-ghost" data-hint={drag.hint.kind} style={{ top: at(drag.start, lane), height: Math.max(((b.end - b.start) / 60_000) * PX, LEAST * PX * 0.7) }} aria-hidden="true">
                      <strong>{isolate(clockText(drag.start, p.zone, p.hour12))} – {isolate(clockText(end, p.zone, p.hour12))}</strong>
                      {drag.hint.kind !== "ok" && <span>{isolate(dropText(drag.hint))}</span>}
                    </div>
                  );
                })()}
              </div>
            );
          })}
        </div>
      </div>
      <div className="sr-only" role="status" aria-live="assertive">
        {isolate(said)}
      </div>
    </div>
  );
}

// A small panel that opens under a button and closes on Escape or a press outside it: a native
// disclosure, so the keyboard and screen readers already know it.

import { type ReactNode, useEffect, useRef } from "react";

export function Pop({ summary, children, align = "start", className = "" }: { summary: ReactNode; children: ReactNode; align?: "start" | "end"; className?: string }) {
  const root = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    const close = (e: Event) => {
      const d = root.current;
      if (d?.open && e.target instanceof Node && !d.contains(e.target)) d.open = false;
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape" && root.current?.open) {
        root.current.open = false;
        root.current.querySelector("summary")?.focus();
      }
    };
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("keydown", esc);
    };
  }, []);
  return (
    <details className={`pop ${className}`} ref={root}>
      <summary className="btn btn-default btn-md">{summary}</summary>
      <div className="pop-body" data-align={align}>
        {children}
      </div>
    </details>
  );
}

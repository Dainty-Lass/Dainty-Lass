"use client";

import { useEffect, useRef } from "react";

/**
 * A console's text, newest at the bottom, following it unless the reader has scrolled up: the server
 * manager's server output and the launcher's Game log. `fill` takes the height it is given.
 */
export function ConsoleView({
  lines,
  emptyText,
  fill = false,
}: {
  lines: string[];
  emptyText: string;
  fill?: boolean;
}) {
  const ref = useRef<HTMLPreElement>(null);
  const pinned = useRef(true);
  // biome-ignore lint/correctness/useExhaustiveDependencies: new lines are the trigger to scroll, not an input.
  useEffect(() => {
    const el = ref.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [lines]);
  return (
    <pre
      ref={ref}
      className="dainty-console"
      data-fill={fill || undefined}
      onScroll={(e) => {
        const el = e.currentTarget;
        pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
      }}
    >
      {lines.length ? lines.join("\n") : emptyText}
    </pre>
  );
}

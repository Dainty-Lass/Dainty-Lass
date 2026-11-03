/**
 * Hourly bars: total in the accent, the flagged part stacked on top in red. Drawn as SVG with token
 * colours from globals.css, so it follows the theme without a chart library.
 */
export function Sparkline({
  points,
  label,
  height = 64,
}: {
  points: { total: number; flagged: number }[];
  label: string;
  height?: number;
}) {
  const max = Math.max(1, ...points.map((p) => p.total));
  const w = 8;
  const gap = 3;
  const width = points.length * (w + gap);
  return (
    <svg
      className="dash-spark"
      viewBox={`0 0 ${width} ${height}`}
      width="100%"
      height={height}
      preserveAspectRatio="none"
      role="img"
      aria-label={label}
    >
      {points.map((p, i) => {
        const h = Math.max(p.total > 0 ? 2 : 0, (p.total / max) * (height - 2));
        const f = (p.flagged / max) * (height - 2);
        const x = i * (w + gap);
        return (
          <g key={x}>
            <rect className="dash-spark-total" x={x} y={height - h} width={w} height={h} rx={2} />
            {f > 0 && (
              <rect
                className="dash-spark-flagged"
                x={x}
                y={height - h}
                width={w}
                height={Math.min(f, h)}
                rx={2}
              />
            )}
          </g>
        );
      })}
    </svg>
  );
}

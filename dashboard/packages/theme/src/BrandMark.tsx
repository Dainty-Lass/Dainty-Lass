/** The launcher icon's mark (brand/icon.png in dainty-lass), as SVG so it follows the theme's size. */
export function BrandMark({ size = 28, label }: { size?: number; label?: string }) {
  return (
    <svg
      className="dainty-brand-mark"
      width={size}
      height={size}
      viewBox="0 0 1024 1024"
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      <rect x="260" y="220" width="141" height="580" />
      <rect x="260" y="660" width="501" height="140" />
      <circle cx="680" cy="310" r="80" />
    </svg>
  );
}

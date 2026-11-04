import type { SVGProps } from "react";

/** Twitch's mark, in currentColor so it takes the button's ink. */
export function TwitchIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      fill="currentColor"
      aria-hidden="true"
      {...props}
    >
      <path d="M11.57 4.71h1.72v5.14h-1.72Zm4.71 0H18v5.14h-1.72ZM6 0 1.71 4.29v15.42h5.15V24l4.28-4.29h3.43L22.29 12V0Zm14.57 11.14-3.43 3.43h-3.43l-3 3v-3H6.86V1.71h13.71Z" />
    </svg>
  );
}

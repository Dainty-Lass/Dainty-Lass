"use client";

import { useCallback, useEffect, useRef, useState } from "react";

export type SplashStage = "spinning" | "settling" | "dismissed";

// The settle: one last turn that decelerates the logos out to the ring.
const SETTLE = {
  begin: "indefinite",
  dur: "2.5s",
  fill: "freeze",
  calcMode: "spline",
  keyTimes: "0;1",
  keySplines: "0.16 1 0.3 1",
} as const;
const SETTLE_MS = 2500;
const LOOP = { dur: "1.4s", repeatCount: "indefinite" } as const;

// Each logo's place on the ring, in degrees from the top, and its colours (dainty-splash-* in brand.css).
const NODES = [
  { angle: 0, tint: "cyan" },
  { angle: 240, tint: "orange" },
  { angle: 120, tint: "green" },
] as const;

/**
 * The boot splash: three logos spin while the page waits on the backend, spiral out to a glowing ring
 * over the revealed page ("settling"), then fade ("dismissed"). It is prerendered with the page, so it
 * shows from the first frame. `onSettled` fires when the ring lights, 2.5s into "settling".
 */
export function SplashLoader({ stage, onSettled }: { stage: SplashStage; onSettled?: () => void }) {
  const svgRef = useRef<SVGSVGElement>(null);
  const loopRef = useRef<SVGAnimateTransformElement>(null);
  const ringFadeRef = useRef<SVGAnimateElement>(null);
  const ringSpinRef = useRef<SVGAnimateTransformElement>(null);
  const onSettledRef = useRef(onSettled);
  const [unmounted, setUnmounted] = useState(false);

  useEffect(() => {
    onSettledRef.current = onSettled;
  }, [onSettled]);

  useEffect(() => {
    const svg = svgRef.current;
    if (stage !== "settling" || !svg) return;
    const loop = loopRef.current;
    let settleTimer: ReturnType<typeof setTimeout> | undefined;
    let started = false;
    const start = () => {
      if (started) return;
      started = true;
      for (const anim of svg.querySelectorAll<SVGAnimationElement>(".dainty-splash-settle")) {
        anim.beginElement();
      }
      settleTimer = setTimeout(() => {
        ringFadeRef.current?.beginElement();
        ringSpinRef.current?.beginElement();
        onSettledRef.current?.();
      }, SETTLE_MS);
    };
    // On a loop boundary (0 deg) so the logos don't jump; the timer covers a webview without repeatEvent.
    loop?.addEventListener("repeatEvent", start, { once: true });
    const fallback = setTimeout(start, loop ? 1450 : 0);
    return () => {
      loop?.removeEventListener("repeatEvent", start);
      clearTimeout(fallback);
      clearTimeout(settleTimer);
    };
  }, [stage]);

  // Out of the DOM once the fade is over.
  useEffect(() => {
    if (stage !== "dismissed") return;
    const timer = setTimeout(() => setUnmounted(true), 700);
    return () => clearTimeout(timer);
  }, [stage]);

  if (unmounted) return null;

  return (
    <div className="dainty-splash" data-stage={stage} aria-hidden="true">
      <svg
        ref={svgRef}
        className="dainty-splash-svg"
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 1200 1150"
        aria-hidden="true"
      >
        <defs>
          <g id="dainty-splash-shards-top">
            <path d="M 447 45 L 527 121 L 541 217 L 445 113 Z" />
            <path d="M 524 52 L 737 276 L 849 360 L 944 397 L 769 421 L 803 444 L 641 487 L 535 636 L 468 541 L 493 610 L 452 706 L 413 601 L 347 495 L 188 439 L 221 417 L 57 399 L 191 330 L 263 266 L 342 305 L 294 237 L 337 198 L 409 301 L 519 366 L 373 171 L 416 134 L 572 311 L 689 381 Z" />
          </g>
          <g id="dainty-splash-shards-bottom">
            <path d="M 58 450 L 186 457 L 259 550 Q 312 560 341 526 L 379 581 L 365 605 Q 330 615 337 653 L 269 622 L 170 513 L 131 517 Z" />
            <path d="M 192 610 L 346 671 L 367 742 L 361 800 Z" />
            <path d="M 393 661 L 443 811 L 469 803 L 518 882 L 512 938 L 497 955 L 385 829 Z" />
            <path d="M 493 777 L 561 754 L 597 661 L 618 742 L 613 798 L 591 859 L 571 881 C 569 855 565 835 557 830 L 511 825 Z" />
            <path d="M 646 691 L 815 583 L 640 796 L 634 779 Z" />
            <path d="M 605 603 L 612 559 L 653 513 L 671 528 L 677 558 C 705 552 730 546 745 539 L 805 470 L 854 472 L 927 454 L 867 514 L 837 511 L 747 601 L 655 668 C 654 642 638 616 605 603 Z" />
          </g>
          {/* Centred on (0,0); each <use> of it picks its colours from its class */}
          <g id="dainty-splash-logo" transform="translate(-165, -165) scale(0.33)">
            <use href="#dainty-splash-shards-top" className="dainty-splash-top" />
            <use href="#dainty-splash-shards-bottom" className="dainty-splash-bottom" />
          </g>

          {/* The ring stops short of each logo: cut by the shards themselves, so their gaps stay clear */}
          <mask id="dainty-splash-cutout">
            <rect width="1200" height="1150" fill="white" />
            <g className="dainty-splash-cutout">
              <use href="#dainty-splash-logo" transform="translate(600, 240)" />
              <use href="#dainty-splash-logo" transform="translate(271, 810)" />
              <use href="#dainty-splash-logo" transform="translate(929, 810)" />
            </g>
          </mask>

          <linearGradient
            id="dainty-splash-cyan-orange"
            x1="600"
            y1="240"
            x2="271"
            y2="810"
            gradientUnits="userSpaceOnUse"
          >
            <stop offset="0%" className="dainty-splash-stop-cyan" />
            <stop offset="100%" className="dainty-splash-stop-orange" />
          </linearGradient>
          <linearGradient
            id="dainty-splash-cyan-green"
            x1="600"
            y1="240"
            x2="929"
            y2="810"
            gradientUnits="userSpaceOnUse"
          >
            <stop offset="0%" className="dainty-splash-stop-cyan" />
            <stop offset="100%" className="dainty-splash-stop-green" />
          </linearGradient>
          <linearGradient
            id="dainty-splash-orange-green"
            x1="271"
            y1="810"
            x2="929"
            y2="810"
            gradientUnits="userSpaceOnUse"
          >
            <stop offset="0%" className="dainty-splash-stop-orange" />
            <stop offset="100%" className="dainty-splash-stop-green" />
          </linearGradient>

          <filter
            id="dainty-splash-glow"
            x="-25%"
            y="-25%"
            width="150%"
            height="150%"
            colorInterpolationFilters="sRGB"
          >
            <feGaussianBlur in="SourceGraphic" stdDeviation="8" result="blur" />
            <feMerge>
              <feMergeNode in="blur" />
              <feMergeNode in="SourceGraphic" />
            </feMerge>
          </filter>
        </defs>

        {/* The ring, hidden until the logos land */}
        <g mask="url(#dainty-splash-cutout)" opacity="0">
          <animate
            ref={ringFadeRef}
            attributeName="opacity"
            from="0"
            to="1"
            begin="indefinite"
            dur="0.45s"
            fill="freeze"
          />
          <g filter="url(#dainty-splash-glow)">
            <g>
              <animateTransform
                ref={ringSpinRef}
                attributeName="transform"
                type="rotate"
                from="0 600 620"
                to="360 600 620"
                begin="indefinite"
                dur="6s"
                repeatCount="indefinite"
              />
              <g className="dainty-splash-ring">
                <path
                  d="M 600 240 A 380 380 0 0 0 271 810"
                  stroke="url(#dainty-splash-cyan-orange)"
                />
                <path
                  d="M 600 240 A 380 380 0 0 1 929 810"
                  stroke="url(#dainty-splash-cyan-green)"
                />
                <path
                  d="M 271 810 A 380 380 0 0 0 929 810"
                  stroke="url(#dainty-splash-orange-green)"
                />
              </g>
            </g>
          </g>
        </g>

        {/* Each logo orbits, then settles; the inner counter-rotation keeps it upright through both */}
        <g transform="translate(600, 620)">
          {NODES.map(({ angle, tint }, i) => (
            <g key={tint}>
              <animateTransform
                ref={i === 0 ? loopRef : undefined}
                attributeName="transform"
                type="rotate"
                from={angle}
                to={angle + 360}
                {...LOOP}
              />
              <animateTransform
                className="dainty-splash-settle"
                attributeName="transform"
                type="rotate"
                from={angle}
                to={angle + 360}
                {...SETTLE}
              />
              <g transform="translate(0, -75)">
                <animateTransform
                  className="dainty-splash-settle"
                  attributeName="transform"
                  type="translate"
                  from="0 -75"
                  to="0 -380"
                  {...SETTLE}
                />
                <g>
                  <animateTransform
                    attributeName="transform"
                    type="rotate"
                    from={-angle}
                    to={-angle - 360}
                    {...LOOP}
                  />
                  <animateTransform
                    className="dainty-splash-settle"
                    attributeName="transform"
                    type="rotate"
                    from={-angle}
                    to={-angle - 360}
                    {...SETTLE}
                  />
                  <g transform="scale(0.65)">
                    <animateTransform
                      className="dainty-splash-settle"
                      attributeName="transform"
                      type="scale"
                      from="0.65"
                      to="1"
                      {...SETTLE}
                    />
                    <use
                      href="#dainty-splash-logo"
                      className={`dainty-splash-${tint}`}
                      filter="url(#dainty-splash-glow)"
                    />
                  </g>
                </g>
              </g>
            </g>
          ))}
        </g>
      </svg>
    </div>
  );
}

/** A desktop page's boot: spins until `ready`, settles over the page, holds the lit ring for a beat, fades. */
export function BootSplash({ ready }: { ready: boolean }) {
  const [settled, setSettled] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const onSettled = useCallback(() => setSettled(true), []);
  useEffect(() => {
    if (!settled) return;
    const timer = setTimeout(() => setDismissed(true), 600);
    return () => clearTimeout(timer);
  }, [settled]);
  const stage: SplashStage = dismissed ? "dismissed" : ready ? "settling" : "spinning";
  return <SplashLoader stage={stage} onSettled={onSettled} />;
}

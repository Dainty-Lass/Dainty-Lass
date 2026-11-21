/** The logo, one shard per element, as the apps' icons draw it (crates/dainty-launcher-app/icons). */
export function BrandMark({ size = 28, label }: { size?: number; label?: string }) {
  return (
    <svg
      className="dainty-brand-mark"
      width={size}
      height={size}
      viewBox="0 0 1000 1000"
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      <path className="dainty-elemental-radiant" d="M 447 45 L 527 121 L 541 217 L 445 113 Z" />
      <path
        className="dainty-elemental-neutral"
        d="M 524 52 L 737 276 L 849 360 L 944 397 L 769 421 L 803 444 L 641 487 L 535 636 L 468 541 L 493 610 L 452 706 L 413 601 L 347 495 L 188 439 L 221 417 L 57 399 L 191 330 L 263 266 L 342 305 L 294 237 L 337 198 L 409 301 L 519 366 L 373 171 L 416 134 L 572 311 L 689 381 Z"
      />
      <path
        className="dainty-elemental-frost"
        d="M 58 450 L 186 457 L 259 550 Q 312 560 341 526 L 379 581 L 365 605 Q 330 615 337 653 L 269 622 L 170 513 L 131 517 Z"
      />
      <path className="dainty-elemental-shock" d="M 192 610 L 346 671 L 367 742 L 361 800 Z" />
      <path
        className="dainty-elemental-terra"
        d="M 393 661 L 443 811 L 469 803 L 518 882 L 512 938 L 497 955 L 385 829 Z"
      />
      <path
        className="dainty-elemental-umbral"
        d="M 493 777 L 561 754 L 597 661 L 618 742 L 613 798 L 591 859 L 571 881 C 569 855 565 835 557 830 L 511 825 Z"
      />
      <path className="dainty-elemental-shock" d="M 646 691 L 815 583 L 640 796 L 634 779 Z" />
      <path
        className="dainty-elemental-blaze"
        d="M 605 603 L 612 559 L 653 513 L 671 528 L 677 558 C 705 552 730 546 745 539 L 805 470 L 854 472 L 927 454 L 867 514 L 837 511 L 747 601 L 655 668 C 654 642 638 616 605 603 Z"
      />
    </svg>
  );
}

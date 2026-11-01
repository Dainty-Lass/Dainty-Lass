/**
 * Astryx variant augmentations. They must name the component's own subpath: core's guards look for
 * the literal interface there. Each needs a `data-variant` rule in variants.css, or it type-checks,
 * renders, and paints nothing.
 */
// Makes this a module, so the block augments `@astryxdesign/core/Button` instead of declaring an
// ambient module that shadows it and breaks every `import {Button}`.
export {};

declare module "@astryxdesign/core/Button" {
  interface ButtonVariantMap {
    /** The accent at a lower weight, for a control that outranks `secondary` where the solid accent is taken. */
    tonal: true;
    /** Pink whatever the accent: the one thing a page needs done next. */
    pink: true;
  }
}

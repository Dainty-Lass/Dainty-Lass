// Drives the boot splash in src/components/splash-loader.astro. Both calls run once, however often they are
// made (a StrictMode remount makes them twice).
let settling: Promise<void> | null = null
let dismissed = false

function animation(id: string): SVGAnimationElement | null {
  const element = document.getElementById(id)
  return element instanceof SVGAnimationElement ? element : null
}

// Spirals the logos out to the ring and shows the page through; resolves when the ring lights, 2.5s on
export function settle(): Promise<void> {
  settling ??= new Promise((resolve) => {
    const overlay = document.getElementById("splash-overlay")
    if (!overlay) return resolve()
    const loop = animation("splash-loop")
    let started = false
    const start = () => {
      if (started) return
      started = true
      loop?.removeEventListener("repeatEvent", start)
      overlay
        .querySelectorAll<SVGAnimationElement>(".settle-anim")
        .forEach((anim) => anim.beginElement())
      overlay.classList.add("ui-revealing")
      setTimeout(() => {
        animation("ring-fade-in")?.beginElement()
        animation("ring-spin")?.beginElement()
        resolve()
      }, 2500)
    }
    // Starts on a loop boundary (0 deg) so the logos don't jump; the timer covers a webview without repeatEvent
    loop?.addEventListener("repeatEvent", start)
    setTimeout(start, loop ? 1450 : 0)
  })
  return settling
}

export function dismiss(): void {
  const overlay = document.getElementById("splash-overlay")
  if (dismissed || !overlay) return
  dismissed = true
  overlay.classList.add("fade-out")
  setTimeout(() => overlay.remove(), 700)
}

import { getSettings } from "@/lib/settings";

/** The checks the Discord bot makes on the link (dainty-discord signup.rs), so one it would drop is refused here. */
export function signupUrl(text: string): string | null {
  if (text.length > 512) return null;
  try {
    const url = new URL(text);
    const ok =
      (url.protocol === "http:" || url.protocol === "https:") &&
      url.hostname !== "" &&
      url.username === "" &&
      url.password === "";
    return ok ? url.toString() : null;
  } catch {
    return null;
  }
}

/**
 * What GET /api/public/signup answers (module-sharing plan A8, D3). It is public, so these two fields
 * and nothing else; on with no usable link reads as off, so the bot falls back to a login invite.
 */
export function signupState(): { enabled: boolean; url: string | null } {
  const { signupEnabled, signupUrl: text } = getSettings();
  const url = signupEnabled ? signupUrl(text) : null;
  return { enabled: url !== null, url };
}

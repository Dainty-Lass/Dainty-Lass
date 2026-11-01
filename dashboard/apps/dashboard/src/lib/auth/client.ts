import { passkeyClient } from "@better-auth/passkey/client";
import { emailOTPClient } from "better-auth/client/plugins";
import { createAuthClient } from "better-auth/react";

/** The browser's half of sign-in; same origin, so no base URL. */
export const authClient = createAuthClient({
  plugins: [emailOTPClient(), passkeyClient()],
});

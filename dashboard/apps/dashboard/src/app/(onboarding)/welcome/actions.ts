"use server";

import { requireSignedIn } from "@/lib/auth/guard";
import { attempt } from "@/lib/errors";
import { completeOnboarding } from "@/lib/onboarding";

export async function completeOnboardingAction(
  displayName: string,
  gameName: string,
  acceptedVersion: number,
) {
  return attempt(async () =>
    completeOnboarding(await requireSignedIn(), { displayName, gameName, acceptedVersion }),
  );
}

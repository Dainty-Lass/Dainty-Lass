"use client";

import { Avatar } from "@astryxdesign/core/Avatar";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Center } from "@astryxdesign/core/Center";
import { CheckboxInput } from "@astryxdesign/core/CheckboxInput";
import { Divider } from "@astryxdesign/core/Divider";
import { Heading } from "@astryxdesign/core/Heading";
import { Markdown } from "@astryxdesign/core/Markdown";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { BrandMark } from "@dainty/theme";
import { useRouter } from "next/navigation";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { signOutAction } from "@/lib/auth/sign-out";
import type { OnboardingResult } from "@/lib/onboarding";
import { completeOnboardingAction } from "./actions";

const NAME = /^[\p{L}\p{N} _.'-]{2,32}$/u;
const GAME_NAME = /^\w{3,16}$/;

/** The one page someone not yet approved sees: their account, the server's terms, and a single button. */
export function WelcomeClient({
  serverName,
  me,
  tos,
  addsRole,
}: {
  serverName: string;
  me: { name: string; avatar?: string; gameName: string; renewing: boolean };
  tos: { text: string; version: number };
  addsRole: boolean;
}) {
  const t = useTranslations("welcome");
  const te = useTranslations("errors");
  const router = useRouter();
  const [displayName, setDisplayName] = useState(me.name);
  const [gameName, setGameName] = useState(me.gameName);
  const [accepted, setAccepted] = useState(false);
  const [busy, setBusy] = useState(false);

  const nameOk = NAME.test(displayName.trim());
  const gameOk = !gameName.trim() || GAME_NAME.test(gameName.trim());

  async function submit() {
    setBusy(true);
    const r = await completeOnboardingAction(displayName, gameName, tos.version);
    setBusy(false);
    if (!r.ok) {
      toast.error(
        r.code === "conflict" ? t("termsChanged") : te(r.code, { detail: r.detail ?? "" }),
      );
      if (r.code === "conflict") router.refresh();
      return;
    }
    const { roleAdded } = r.value as OnboardingResult;
    if (roleAdded === false) toast.warning(t("roleNotAdded"));
    else toast.success(t("done"));
    router.push("/");
    router.refresh();
  }

  return (
    <Center minHeight="100dvh" padding={4}>
      <Stack gap={4} width={640} maxWidth="100%">
        <Stack direction="horizontal" gap={3} align="center">
          <BrandMark size={44} />
          <Stack gap={0}>
            <Heading level={1}>
              {me.renewing ? t("titleRenew") : t("heading", { server: serverName })}
            </Heading>
            <Text type="supporting">{me.renewing ? t("subtitleRenew") : t("subtitle")}</Text>
          </Stack>
        </Stack>

        <Card padding={5}>
          <Stack gap={4}>
            <Stack direction="horizontal" gap={3} align="center">
              <Avatar name={me.name} src={me.avatar} size="md" />
              <Stack gap={0}>
                <Text weight="semibold">{t("account")}</Text>
                <Text type="supporting">{t("accountHint")}</Text>
              </Stack>
            </Stack>
            <TextInput
              label={t("displayName")}
              description={t("displayNameHint")}
              value={displayName}
              onChange={setDisplayName}
              isRequired
              status={
                displayName && !nameOk
                  ? { type: "error", message: t("displayNameInvalid") }
                  : undefined
              }
            />
            <TextInput
              label={t("gameName")}
              description={t("gameNameHint")}
              value={gameName}
              onChange={setGameName}
              isOptional
              status={!gameOk ? { type: "error", message: t("gameNameInvalid") } : undefined}
            />
          </Stack>
        </Card>

        <Card padding={5}>
          <Stack gap={3}>
            <Stack direction="horizontal" justify="between" align="center">
              <Heading level={2}>{t("terms")}</Heading>
              <Text type="supporting">{t("version", { version: tos.version })}</Text>
            </Stack>
            {tos.text ? (
              <Markdown headingLevelStart={3} density="compact">
                {tos.text}
              </Markdown>
            ) : (
              <Banner status="info" title={t("noTerms")} collapsible={false} />
            )}
            <Divider />
            <CheckboxInput
              label={t("accept", { server: serverName })}
              value={accepted}
              onChange={setAccepted}
            />
          </Stack>
        </Card>

        {addsRole && <Text type="supporting">{t("roleHint")}</Text>}
        <Stack direction="horizontal" gap={2} justify="between" align="center">
          <form action={signOutAction}>
            <Button label={t("signOut")} type="submit" variant="ghost" />
          </form>
          <Button
            label={me.renewing ? t("acceptRenew") : t("create")}
            variant="primary"
            size="lg"
            isDisabled={!accepted || !nameOk || !gameOk}
            isLoading={busy}
            onClick={submit}
          />
        </Stack>
      </Stack>
    </Center>
  );
}

"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Token } from "@astryxdesign/core/Token";
import { KeyRound, Mail } from "lucide-react";
import { useRouter } from "next/navigation";
import { useTranslations } from "next-intl";
import { type ReactNode, useState } from "react";
import { toast } from "sonner";
import { AppearanceSettings } from "@/components/AppearanceSettings";
import { DiscordIcon } from "@/components/DiscordIcon";
import { Page } from "@/components/Page";
import { TwitchIcon } from "@/components/TwitchIcon";
import { authClient } from "@/lib/auth/client";
import { unlinkAccountAction } from "./actions";

type Provider = "discord" | "twitch";
const ICONS: Record<Provider, ReactNode> = { discord: <DiscordIcon />, twitch: <TwitchIcon /> };

export function AccountClient({
  email,
  emailVerified,
  accounts,
  passkeys,
  providers,
  linkError,
}: {
  email: string | null;
  emailVerified: boolean;
  accounts: { id: number; providerId: string; accountId: string; createdAt: number }[];
  passkeys: {
    id: string;
    name: string | null;
    deviceType: string;
    backedUp: boolean;
    createdAt: number | null;
  }[];
  providers: { discord: boolean; twitch: boolean; email: boolean };
  linkError: string | null;
}) {
  const t = useTranslations("account");
  const router = useRouter();
  const [busy, setBusy] = useState<string | null>(null);
  const [passkeyName, setPasskeyName] = useState("");

  // One way in must always remain, or the account could never be entered again.
  const ways =
    accounts.length + passkeys.length + (email && emailVerified && providers.email ? 1 : 0);
  const done = (ok: boolean, message: string) => {
    setBusy(null);
    if (ok) {
      toast.success(message);
      router.refresh();
    } else toast.error(t("failed"));
  };

  async function link(provider: Provider) {
    setBusy(provider);
    const r = await authClient.linkSocial({
      provider,
      callbackURL: "/account",
      errorCallbackURL: "/account",
    });
    if (r.error) done(false, "");
  }

  async function unlink(account: { id: number; providerId: string }) {
    setBusy(`unlink-${account.providerId}`);
    const r = await unlinkAccountAction(account.id);
    done(r.ok, t("unlinked"));
  }

  async function addPasskey() {
    setBusy("passkey");
    const r = await authClient.passkey.addPasskey({ name: passkeyName.trim() || undefined });
    if (!r?.error) setPasskeyName("");
    done(!r?.error, t("passkeyAdded"));
  }

  async function removePasskey(id: string) {
    setBusy(`passkey-${id}`);
    const r = await authClient.passkey.deletePasskey({ id });
    done(!r.error, t("passkeyRemoved"));
  }

  const ta = useTranslations("appearance");
  const linked = (p: Provider) => accounts.find((a) => a.providerId === p);

  return (
    <Page title={t("title")} description={t("description")} width={840}>
      <Stack gap={4}>
        {linkError && <Banner status="error" title={t("linkFailed")} collapsible={false} />}

        <Card padding={4}>
          <Stack gap={3}>
            <Heading level={2}>{t("signIns")}</Heading>
            <Text type="supporting">{t("signInsHint")}</Text>
            {(["discord", "twitch"] as const)
              .filter((p) => providers[p] || linked(p))
              .map((p) => {
                const account = linked(p);
                return (
                  <Stack key={p} direction="horizontal" gap={3} align="center" justify="between">
                    <Stack direction="horizontal" gap={2} align="center">
                      {ICONS[p]}
                      <Stack gap={0}>
                        <Text weight="medium">{t(`providers.${p}`)}</Text>
                        <Text type="supporting">
                          {account
                            ? t("linkedSince", { date: new Date(account.createdAt) })
                            : t("notLinked")}
                        </Text>
                      </Stack>
                    </Stack>
                    {account ? (
                      <Button
                        label={t("unlink")}
                        variant="ghost"
                        size="sm"
                        isDisabled={ways <= 1}
                        tooltip={ways <= 1 ? t("lastWay") : undefined}
                        isLoading={busy === `unlink-${p}`}
                        onClick={() => unlink(account)}
                      />
                    ) : (
                      <Button
                        label={t("link")}
                        variant="secondary"
                        size="sm"
                        isLoading={busy === p}
                        onClick={() => link(p)}
                      />
                    )}
                  </Stack>
                );
              })}
            <Stack direction="horizontal" gap={3} align="center" justify="between">
              <Stack direction="horizontal" gap={2} align="center">
                <Mail size={16} />
                <Stack gap={0}>
                  <Text weight="medium">{t("providers.email")}</Text>
                  <Text type="supporting">{email ?? t("noEmail")}</Text>
                </Stack>
              </Stack>
              {email && (
                <Token
                  size="sm"
                  label={emailVerified ? t("verified") : t("unverified")}
                  color={emailVerified ? "green" : "gray"}
                />
              )}
            </Stack>
          </Stack>
        </Card>

        <Card padding={4}>
          <Stack gap={3}>
            <Heading level={2}>{t("passkeys")}</Heading>
            <Text type="supporting">{t("passkeysHint")}</Text>
            {passkeys.length === 0 ? (
              <EmptyState isCompact title={t("noPasskeys")} />
            ) : (
              passkeys.map((p) => (
                <Stack key={p.id} direction="horizontal" gap={3} align="center" justify="between">
                  <Stack direction="horizontal" gap={2} align="center">
                    <KeyRound size={16} />
                    <Stack gap={0}>
                      <Text weight="medium">{p.name || t("unnamedPasskey")}</Text>
                      <Text type="supporting">
                        {p.backedUp ? t("synced") : t("deviceBound")}
                        {p.createdAt && (
                          <>
                            {" · "}
                            <Timestamp value={p.createdAt} format="relative" />
                          </>
                        )}
                      </Text>
                    </Stack>
                  </Stack>
                  <Button
                    label={t("remove")}
                    variant="ghost"
                    size="sm"
                    isDisabled={ways <= 1}
                    tooltip={ways <= 1 ? t("lastWay") : undefined}
                    isLoading={busy === `passkey-${p.id}`}
                    onClick={() => removePasskey(p.id)}
                  />
                </Stack>
              ))
            )}
            <Stack direction="horizontal" gap={2} align="end" wrap="wrap">
              <TextInput
                label={t("passkeyName")}
                description={t("passkeyNameHint")}
                value={passkeyName}
                onChange={setPasskeyName}
                isOptional
              />
              <Button
                label={t("addPasskey")}
                variant="primary"
                icon={<KeyRound />}
                isLoading={busy === "passkey"}
                onClick={addPasskey}
              />
            </Stack>
          </Stack>
        </Card>

        <Card padding={4}>
          <Stack gap={3}>
            <Stack gap={1}>
              <Heading level={2}>{ta("title")}</Heading>
              <Text type="supporting">{ta("hint")}</Text>
            </Stack>
            <AppearanceSettings />
          </Stack>
        </Card>
      </Stack>
    </Page>
  );
}

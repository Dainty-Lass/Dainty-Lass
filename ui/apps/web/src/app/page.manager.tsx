"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Grid } from "@astryxdesign/core/Grid";
import { Heading } from "@astryxdesign/core/Heading";
import { MetadataList, MetadataListItem } from "@astryxdesign/core/MetadataList";
import { NumberInput } from "@astryxdesign/core/NumberInput";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Switch } from "@astryxdesign/core/Switch";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { Token } from "@astryxdesign/core/Token";
import { useBridge } from "@dainty/bridge/react";
import { Copy, Play, RefreshCw, Square } from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { ConsoleView as Console } from "@/components/desktop/ConsoleView";
import { ManagerShell, STATE_DOT, useAct } from "@/components/manager/ManagerShell";

export default function ServerPage() {
  const t = useTranslations("manager.server");
  const { api, kit, server } = useBridge();
  const act = useAct();
  const [inviteFor, setInviteFor] = useState("");
  const config = kit?.serverConfig;
  const [boost, setBoost] = useState(false);
  const [xp, setXp] = useState(1);
  const [rewards, setRewards] = useState(1);

  useEffect(() => {
    setBoost(Boolean(config?.multipliers));
    setXp(config?.multipliers?.xp ?? 1);
    setRewards(config?.multipliers?.rewards ?? 1);
  }, [config?.multipliers]);

  const state = server?.state ?? "stopped";
  const busy = state === "starting" || state === "stopping";
  const up = state === "running" || state === "external";

  return (
    <ManagerShell
      title={t("title")}
      description={
        config?.configured
          ? t("description", {
              name: config.name ?? "",
              mode: t(`modes.${config.mode ?? "private"}`),
            })
          : t("notConfigured")
      }
      actions={
        up ? (
          <Button
            label={t("stop")}
            variant="destructive"
            icon={<Square size={16} />}
            isLoading={busy}
            isDisabled={state === "external"}
            clickAction={async () => void (await act(() => api!.hostServerStop()))}
          />
        ) : (
          <Button
            label={t("start")}
            variant="primary"
            icon={<Play size={16} />}
            isLoading={busy}
            isDisabled={!config?.configured}
            clickAction={async () => void (await act(() => api!.hostServerStart()))}
          />
        )
      }
    >
      {state === "failed" && server?.message && (
        <Banner
          status="error"
          title={t("failed")}
          description={server.message}
          collapsible={false}
        />
      )}
      {state === "external" && <Banner status="info" title={t("external")} collapsible={false} />}
      {!config?.configured && (
        <Banner
          status="warning"
          title={t("setupFirst")}
          collapsible={false}
          endContent={<Button label={t("openSetup")} size="sm" href="/setup/" />}
        />
      )}

      <Grid columns={{ minWidth: 320, repeat: "fit" }} gap={4}>
        <Card padding={4}>
          <Stack gap={3}>
            <Stack direction="horizontal" gap={2} align="center">
              <StatusDot
                variant={STATE_DOT[state]}
                label={t(`states.${state}`)}
                isPulsing={state === "running"}
              />
              <Heading level={2}>{t(`states.${state}`)}</Heading>
            </Stack>
            <MetadataList label={{ position: "start", width: 140 }}>
              <MetadataListItem label={t("name")}>{config?.name ?? "—"}</MetadataListItem>
              <MetadataListItem label={t("mode")}>
                {config?.mode ? t(`modes.${config.mode}`) : "—"}
              </MetadataListItem>
              {config?.mode === "public" && (
                <MetadataListItem label={t("address")}>{config.host ?? "—"}</MetadataListItem>
              )}
              <MetadataListItem label={t("gameData")}>
                <Stack direction="horizontal" gap={1}>
                  <Token
                    size="sm"
                    label={t("backendData", {
                      state: t(`dataStates.${kit?.gameData.backend ?? "missing"}`),
                    })}
                    color={kit?.gameData.backend === "real" ? "green" : "orange"}
                  />
                  <Token
                    size="sm"
                    label={t("directorData", {
                      state: t(`dataStates.${kit?.gameData.director ?? "missing"}`),
                    })}
                    color={kit?.gameData.director === "real" ? "green" : "orange"}
                  />
                </Stack>
              </MetadataListItem>
            </MetadataList>
          </Stack>
        </Card>

        <Card padding={4}>
          <Stack gap={3}>
            <Heading level={2}>{t("invites")}</Heading>
            <Text type="supporting">{t("invitesHint")}</Text>
            <TextInput
              label={t("inviteFor")}
              value={inviteFor}
              onChange={setInviteFor}
              isOptional
              placeholder={t("inviteForPlaceholder")}
              width="100%"
            />
            <Stack direction="horizontal" justify="end">
              <Button
                label={t("createInvite")}
                isDisabled={!config?.configured}
                clickAction={async () =>
                  void (await act(() => api!.hostCreateInvite(inviteFor.trim() || null)))
                }
              />
            </Stack>
            {server?.invite && (
              <Stack direction="horizontal" gap={2} align="center">
                <Text type="code" maxLines={1}>
                  {server.invite.link}
                </Text>
                <Button
                  label={t("copy")}
                  size="sm"
                  icon={<Copy size={14} />}
                  clickAction={async () => {
                    if (await api!.copyText(server.invite!.link)) toast.success(t("copied"));
                  }}
                />
              </Stack>
            )}
          </Stack>
        </Card>
      </Grid>

      <Card padding={0}>
        <Stack
          direction="horizontal"
          justify="between"
          align="center"
          paddingInline={4}
          paddingBlock={3}
        >
          <Heading level={2}>{t("console")}</Heading>
          <Button
            label={t("openLogs")}
            size="sm"
            variant="ghost"
            clickAction={async () => void (await act(() => api!.hostOpen("host_logs")))}
          />
        </Stack>
        <Stack paddingInline={4} paddingBlockEnd={4}>
          <Console lines={server?.lines ?? []} emptyText={t("consoleEmpty")} />
        </Stack>
      </Card>

      <Grid columns={{ minWidth: 320, repeat: "fit" }} gap={4}>
        <Card padding={4}>
          <Stack gap={3}>
            <Heading level={2}>{t("multipliers")}</Heading>
            <Text type="supporting">
              {config?.mode === "public" ? t("multipliersPrivateOnly") : t("multipliersHint")}
            </Text>
            <Switch
              label={t("boost")}
              value={boost}
              onChange={setBoost}
              isDisabled={config?.mode === "public"}
            />
            {boost && (
              <Stack direction="horizontal" gap={3}>
                <NumberInput
                  label={t("xp")}
                  value={xp}
                  onChange={(v) => setXp(Math.max(1, Math.min(100, v)))}
                />
                <NumberInput
                  label={t("rewards")}
                  value={rewards}
                  onChange={(v) => setRewards(Math.max(1, Math.min(100, v)))}
                />
              </Stack>
            )}
            <Stack direction="horizontal" justify="end">
              <Button
                label={t("apply")}
                isDisabled={config?.mode === "public"}
                clickAction={async () =>
                  void (await act(
                    () => api!.hostSetMultipliers(boost ? { xp, rewards } : null),
                    t("applied"),
                  ))
                }
              />
            </Stack>
          </Stack>
        </Card>

        <Card padding={4}>
          <Stack gap={3}>
            <Heading level={2}>{t("update")}</Heading>
            <Text type="supporting">{t("updateHint")}</Text>
            {kit?.update && (
              <Banner
                status={kit.update.runtimeChanged ? "warning" : "success"}
                title={
                  kit.update.state === "updated"
                    ? t("updated", { files: kit.update.files, head: kit.update.head })
                    : t("upToDate", { head: kit.update.head })
                }
                description={kit.update.runtimeChanged ? t("runtimeChanged") : undefined}
                collapsible={false}
              />
            )}
            <Stack direction="horizontal" justify="end">
              <Button
                label={t("checkUpdate")}
                icon={<RefreshCw size={16} />}
                isDisabled={up}
                tooltip={up ? t("stopToUpdate") : undefined}
                clickAction={async () => void (await act(() => api!.hostUpdate()))}
              />
            </Stack>
          </Stack>
        </Card>
      </Grid>
    </ManagerShell>
  );
}

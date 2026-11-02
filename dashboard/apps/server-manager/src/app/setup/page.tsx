"use client";

import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { SegmentedControl, SegmentedControlItem } from "@astryxdesign/core/SegmentedControl";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import type { ExternalTarget, HostServerMode, RequirementId } from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { Download, FolderOpen } from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { ManagerShell, useAct } from "@/components/ManagerShell";

// Where each missing requirement is fetched from: only targets the bridge's open_external allows.
const GET: Partial<Record<RequirementId, ExternalTarget>> = {
  git: "git",
  node: "nodejs",
  mongodb: "mongodb",
  tailscale: "tailscale_download",
  visual_studio: "visual_studio_installer",
  rust: "rust_installer",
  dumper7: "dumper7_zip",
  vc_redist: "vc_redist",
};

const STATE = { ok: "success", missing: "error", problem: "warning" } as const;

export default function SetupPage() {
  const t = useTranslations("setup");
  const { api, kit, snapshot } = useBridge();
  const act = useAct();
  const [name, setName] = useState("");
  const [mode, setMode] = useState<HostServerMode>("private");
  const [publicHost, setPublicHost] = useState("");
  const hide = snapshot?.settings.streamerMode;

  useEffect(() => {
    if (!kit) return;
    setName(kit.serverConfig.name ?? "");
    setMode(kit.serverConfig.mode ?? "private");
    setPublicHost(kit.serverConfig.host ?? "");
  }, [kit?.serverConfig.name, kit?.serverConfig.mode, kit?.serverConfig.host, kit]);

  const ready = name.trim().length > 0 && (mode === "private" || publicHost.trim().length > 0);

  return (
    <ManagerShell title={t("title")} description={t("description")}>
      <Card padding={0}>
        <Stack paddingInline={4} paddingBlock={3} gap={1}>
          <Heading level={2}>{t("requirements")}</Heading>
          <Text type="supporting">{t("requirementsHint")}</Text>
        </Stack>
        <List hasDividers>
          {kit?.requirements.map((r) => (
            <ListItem
              key={r.id}
              label={t(`reqs.${r.id}`)}
              description={
                r.state === "problem" && r.problem
                  ? t(`problems.${r.problem as "too_old"}`)
                  : (r.detail ?? undefined)
              }
              startContent={<StatusDot variant={STATE[r.state]} label={t(`states.${r.state}`)} />}
              endContent={
                r.state === "ok" ? (
                  <Badge variant="success" label={t("states.ok")} />
                ) : GET[r.id] ? (
                  <Button
                    label={t("get")}
                    size="sm"
                    icon={<Download size={14} />}
                    clickAction={async () => void (await act(() => api!.openExternal(GET[r.id]!)))}
                  />
                ) : (
                  <Badge variant="error" label={t(`states.${r.state}`)} />
                )
              }
            />
          ))}
        </List>
      </Card>

      <Card padding={4}>
        <Stack gap={3}>
          <Heading level={2}>{t("workspace")}</Heading>
          <Text type="supporting">
            {kit?.workspace ? (hide ? t("hiddenPath") : kit.workspace) : t("noWorkspace")}
          </Text>
          {kit && !kit.workspaceOk && kit.workspace && (
            <Text color="accent">{t("workspaceInvalid")}</Text>
          )}
          <Stack direction="horizontal" gap={2} wrap="wrap">
            <Button
              label={t("chooseWorkspace")}
              icon={<FolderOpen size={16} />}
              clickAction={async () => void (await act(() => api!.hostChooseWorkspace()))}
            />
            <Button
              label={t("downloadProject")}
              icon={<Download size={16} />}
              clickAction={async () => void (await act(() => api!.hostDownloadProject()))}
            />
            {kit?.workspace && (
              <Button
                label={t("open")}
                variant="ghost"
                clickAction={async () => void (await act(() => api!.hostOpen("workspace")))}
              />
            )}
          </Stack>
          {kit?.projectTarget && !kit.workspace && (
            <Text type="supporting">
              {t("downloadTarget", { path: hide ? t("hiddenPath") : kit.projectTarget })}
            </Text>
          )}
        </Stack>
      </Card>

      <Card padding={4}>
        <Stack gap={4}>
          <Stack gap={1}>
            <Heading level={2}>{t("server")}</Heading>
            <Text type="supporting">{t("serverHint")}</Text>
          </Stack>
          <TextInput label={t("name")} value={name} onChange={setName} isRequired />
          <SegmentedControl
            label={t("mode")}
            value={mode}
            onChange={(v) => setMode(v as HostServerMode)}
          >
            <SegmentedControlItem value="private" label={t("modes.private")} />
            <SegmentedControlItem value="public" label={t("modes.public")} />
          </SegmentedControl>
          <Text type="supporting">{t(`modeHints.${mode}`)}</Text>
          {mode === "public" && (
            <TextInput
              label={t("publicHost")}
              description={t("publicHostHint")}
              value={publicHost}
              onChange={setPublicHost}
              isRequired
            />
          )}
          <Stack direction="horizontal" justify="end">
            <Button
              label={kit?.serverConfig.configured ? t("reconfigure") : t("configure")}
              variant="primary"
              isDisabled={!ready || !kit?.workspaceOk}
              clickAction={async () =>
                void (await act(
                  () =>
                    api!.hostSetup({
                      name: name.trim(),
                      mode,
                      publicHost: mode === "public" ? publicHost.trim() : null,
                    }),
                  t("configured"),
                ))
              }
            />
          </Stack>
        </Stack>
      </Card>
    </ManagerShell>
  );
}

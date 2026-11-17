"use client";

import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { ProgressBar } from "@astryxdesign/core/ProgressBar";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Text } from "@astryxdesign/core/Text";
import type {
  ExternalTarget,
  HostAction,
  HostFolder,
  HostKit,
  RequirementId,
} from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { Download, FolderOpen, RefreshCw } from "lucide-react";
import { useTranslations } from "next-intl";
import type { ReactNode } from "react";
import { LauncherShell } from "@/components/launcher/LauncherShell";
import { refreshHostKit, useHostKit } from "@/components/launcher/useHostKit";
import { useAct } from "@/components/manager/ManagerShell";

// Only targets the bridge's open_external allows.
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
const DATA = { real: "success", synthetic: "warning", missing: "error" } as const;
const JOB = { running: "info", done: "success", failed: "error" } as const;

function Section({
  title,
  hint,
  folders,
  children,
}: {
  title: string;
  hint: string;
  folders?: HostFolder[];
  children: ReactNode;
}) {
  const t = useTranslations("launcher.host");
  const { api } = useBridge();
  const act = useAct();
  return (
    <Card padding={0}>
      <Stack paddingInline={4} paddingBlock={3} gap={1}>
        <Stack direction="horizontal" justify="between" align="center" wrap="wrap" gap={2}>
          <Heading level={2}>{title}</Heading>
          {folders && (
            <Stack direction="horizontal" gap={1} wrap="wrap">
              {folders.map((f) => (
                <Button
                  key={f}
                  label={t(`folders.${f}`)}
                  variant="ghost"
                  size="sm"
                  icon={<FolderOpen size={14} />}
                  clickAction={async () => void (await act(() => api!.hostOpen(f)))}
                />
              ))}
            </Stack>
          )}
        </Stack>
        <Text type="supporting">{hint}</Text>
      </Stack>
      <List hasDividers>{children}</List>
    </Card>
  );
}

/** One host_action: what it does, whether its output is there, and its button. */
function Step({
  action,
  kit,
  done,
  needsWorkspace = true,
  primary = false,
}: {
  action: HostAction;
  kit: HostKit;
  done?: boolean;
  needsWorkspace?: boolean;
  primary?: boolean;
}) {
  const t = useTranslations("launcher.host");
  const { api } = useBridge();
  const act = useAct();
  const busy = kit.job?.state === "running";
  return (
    <ListItem
      label={t(`actions.${action}.label`)}
      description={t(`actions.${action}.hint`)}
      startContent={
        done === undefined ? undefined : (
          <StatusDot
            variant={done ? "success" : "neutral"}
            label={t(done ? "present" : "absent")}
          />
        )
      }
      endContent={
        <Button
          label={t(`actions.${action}.button`)}
          size="sm"
          variant={primary ? "primary" : "secondary"}
          isDisabled={busy || (needsWorkspace && !kit.workspaceOk)}
          clickAction={async () => void (await act(() => api!.hostAction(action)))}
        />
      }
    />
  );
}

function Job({ kit, hide }: { kit: HostKit; hide: boolean }) {
  const t = useTranslations("launcher.host");
  const { api } = useBridge();
  const act = useAct();
  const job = kit.job;
  if (!job) return null;
  const kind = t(`jobs.${job.kind}`);
  return (
    <Card padding={4}>
      <Stack gap={3}>
        <Stack direction="horizontal" justify="between" align="center" gap={2}>
          <Heading level={2}>{kind}</Heading>
          <Badge variant={JOB[job.state]} label={t(`jobStates.${job.state}`)} />
        </Stack>
        <ProgressBar
          label={kind}
          isLabelHidden
          isIndeterminate={job.state === "running"}
          value={job.state === "running" ? 0 : 100}
          variant={job.state === "failed" ? "error" : job.state === "done" ? "success" : "accent"}
        />
        {job.message && (
          <Text type="code" color="secondary">
            {hide ? t("hiddenPath") : job.message}
          </Text>
        )}
        {job.logFile && (
          <Stack direction="horizontal" justify="end">
            <Button
              label={t("openLogs")}
              variant="ghost"
              size="sm"
              icon={<FolderOpen size={14} />}
              clickAction={async () => void (await act(() => api!.hostOpen("host_logs")))}
            />
          </Stack>
        )}
      </Stack>
    </Card>
  );
}

export default function HostPage() {
  const t = useTranslations("launcher.host");
  const { api, snapshot } = useBridge();
  const kit = useHostKit();
  const act = useAct();
  const enabled = !!snapshot?.app.hostToolsAvailable && !!snapshot.settings.hostTools;
  const hide = !!snapshot?.settings.streamerMode;
  const busy = kit?.job?.state === "running";
  const newest = kit?.dumps[0];

  return (
    <LauncherShell>
      <Stack gap={4} padding={6} isScrollable height="100%">
        <Stack direction="horizontal" justify="between" align="center" gap={2} wrap="wrap">
          <Stack gap={1}>
            <Heading level={1}>{t("title")}</Heading>
            <Text type="supporting">{t("description")}</Text>
          </Stack>
          {enabled && (
            <Button
              label={t("recheck")}
              icon={<RefreshCw size={16} />}
              isDisabled={!api}
              clickAction={async () => void (api && (await refreshHostKit(api)))}
            />
          )}
        </Stack>

        {!enabled ? (
          <EmptyState title={t("off")} description={t("offHint")} />
        ) : !kit ? (
          <EmptyState title={t("loading")} />
        ) : (
          <>
            <Job kit={kit} hide={hide} />

            <Card padding={4}>
              <Stack gap={3}>
                <Heading level={2}>{t("workspace")}</Heading>
                <Text type="supporting">
                  {kit.workspace ? (hide ? t("hiddenPath") : kit.workspace) : t("noWorkspace")}
                </Text>
                {kit.workspace && !kit.workspaceOk && (
                  <Banner status="warning" title={t("workspaceInvalid")} collapsible={false} />
                )}
                <Stack direction="horizontal" gap={2} wrap="wrap">
                  <Button
                    label={t("chooseWorkspace")}
                    icon={<FolderOpen size={16} />}
                    isDisabled={busy}
                    clickAction={async () => void (await act(() => api!.hostChooseWorkspace()))}
                  />
                  <Button
                    label={t("downloadProject")}
                    icon={<Download size={16} />}
                    isDisabled={busy}
                    clickAction={async () => void (await act(() => api!.hostDownloadProject()))}
                  />
                  {kit.workspace && (
                    <Button
                      label={t("open")}
                      variant="ghost"
                      clickAction={async () => void (await act(() => api!.hostOpen("workspace")))}
                    />
                  )}
                </Stack>
                {kit.projectTarget && !kit.workspace && (
                  <Text type="supporting">
                    {t("downloadTarget", { path: hide ? t("hiddenPath") : kit.projectTarget })}
                  </Text>
                )}
              </Stack>
            </Card>

            <Section title={t("sdk")} hint={t("sdkHint")} folders={["dumps", "runtime_source"]}>
              <ListItem
                label={t("dumper")}
                description={
                  kit.dumperDll ? (hide ? t("hiddenPath") : kit.dumperDll) : t("noDumper")
                }
                startContent={
                  <StatusDot
                    variant={kit.dumperDll ? "success" : "neutral"}
                    label={t(kit.dumperDll ? "present" : "absent")}
                  />
                }
                endContent={
                  <Button
                    label={t("chooseDumper")}
                    size="sm"
                    isDisabled={busy}
                    clickAction={async () => void (await act(() => api!.hostChooseDumper()))}
                  />
                }
              />
              <Step action="inject_dumper" kit={kit} needsWorkspace={false} />
              <ListItem
                label={t("dumps")}
                description={
                  newest
                    ? t("newestDump", { name: newest.name, count: kit.dumps.length })
                    : t("noDumps")
                }
              />
              <Step action="import_sdk" kit={kit} done={kit.sdk.present} />
              <ListItem
                label={t("importSdkFolder")}
                description={
                  kit.sdk.present
                    ? t("sdkFiles", { files: kit.sdk.functionFiles })
                    : t("importSdkFolderHint")
                }
                endContent={
                  <Button
                    label={t("chooseFolder")}
                    size="sm"
                    isDisabled={busy || !kit.workspaceOk}
                    clickAction={async () => void (await act(() => api!.hostImportSdkFolder()))}
                  />
                }
              />
            </Section>

            <Section
              title={t("runtime")}
              hint={t("runtimeHint")}
              folders={["runtime_source", "runtime_publish"]}
            >
              <Step action="build_runtime" kit={kit} done={kit.builds.runtime} />
              <Step action="build_loader" kit={kit} done={kit.builds.loader} />
              <Step action="publish_runtime" kit={kit} done={kit.builds.published} />
            </Section>

            <Section
              title={t("gameData")}
              hint={t("gameDataHint")}
              folders={["game_data", "gamedata_packs"]}
            >
              <ListItem
                label={t("dataState")}
                endContent={
                  <Stack direction="horizontal" gap={2} wrap="wrap">
                    <Badge
                      variant={DATA[kit.gameData.backend]}
                      label={t("backendData", { state: t(`dataStates.${kit.gameData.backend}`) })}
                    />
                    <Badge
                      variant={DATA[kit.gameData.director]}
                      label={t("directorData", { state: t(`dataStates.${kit.gameData.director}`) })}
                    />
                  </Stack>
                }
              />
              <Step action="build_exporter" kit={kit} done={kit.builds.exporter} />
              <Step action="inject_exporter" kit={kit} primary />
              <Step action="copy_exports" kit={kit} done={kit.exports.present} />
              <Step action="generate_gamedata" kit={kit} done={kit.gameData.backend === "real"} />
              <Step action="synthetic_data" kit={kit} />
              <Step action="export_gamedata" kit={kit} />
              <ListItem
                label={t("importGameData")}
                description={t("importGameDataHint")}
                endContent={
                  <Button
                    label={t("chooseFile")}
                    size="sm"
                    isDisabled={busy || !kit.workspaceOk}
                    clickAction={async () => void (await act(() => api!.hostImportGameData()))}
                  />
                }
              />
            </Section>

            <Section title={t("requirements")} hint={t("requirementsHint")}>
              {kit.requirements.map((r) => (
                <ListItem
                  key={r.id}
                  label={t(`reqs.${r.id}`)}
                  description={
                    r.state === "problem" && r.problem
                      ? t(`problems.${r.problem as "too_old"}`)
                      : (r.detail ?? undefined)
                  }
                  startContent={
                    <StatusDot variant={STATE[r.state]} label={t(`states.${r.state}`)} />
                  }
                  endContent={
                    r.state === "ok" ? (
                      <Badge variant="success" label={t("states.ok")} />
                    ) : GET[r.id] ? (
                      <Button
                        label={t("get")}
                        size="sm"
                        icon={<Download size={14} />}
                        clickAction={async () =>
                          void (await act(() => api!.openExternal(GET[r.id]!)))
                        }
                      />
                    ) : (
                      <Badge variant="error" label={t(`states.${r.state}`)} />
                    )
                  }
                />
              ))}
            </Section>
          </>
        )}
      </Stack>
    </LauncherShell>
  );
}

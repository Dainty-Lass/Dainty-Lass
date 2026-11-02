"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Text } from "@astryxdesign/core/Text";
import type { HostAction, HostFolder, HostKit } from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { FolderOpen } from "lucide-react";
import { useTranslations } from "next-intl";
import type { ReactNode } from "react";
import { ManagerShell, useAct } from "@/components/ManagerShell";

interface Step {
  key: string;
  done: (kit: HostKit) => boolean;
  run: HostAction | "choose_dumper" | "import_sdk_folder" | "import_gamedata";
  folder?: HostFolder;
}

// The host kit's pipeline in the order a first-time host works through it.
const STAGES: { key: "sdk" | "runtime" | "data"; steps: Step[] }[] = [
  {
    key: "sdk",
    steps: [
      { key: "dumper", done: (k) => Boolean(k.dumperDll), run: "choose_dumper" },
      {
        key: "inject_dumper",
        done: (k) => k.dumps.length > 0,
        run: "inject_dumper",
        folder: "dumps",
      },
      {
        key: "import_sdk",
        done: (k) => k.sdk.present,
        run: "import_sdk",
        folder: "runtime_source",
      },
      { key: "import_sdk_folder", done: (k) => k.sdk.present, run: "import_sdk_folder" },
    ],
  },
  {
    key: "runtime",
    steps: [
      { key: "build_runtime", done: (k) => k.builds.runtime, run: "build_runtime" },
      { key: "build_loader", done: (k) => k.builds.loader, run: "build_loader" },
      {
        key: "publish_runtime",
        done: (k) => k.builds.published,
        run: "publish_runtime",
        folder: "runtime_publish",
      },
    ],
  },
  {
    key: "data",
    steps: [
      { key: "build_exporter", done: (k) => k.builds.exporter, run: "build_exporter" },
      { key: "inject_exporter", done: (k) => k.exports.inGame, run: "inject_exporter" },
      { key: "copy_exports", done: (k) => k.exports.present, run: "copy_exports" },
      {
        key: "generate_gamedata",
        done: (k) => k.gameData.backend === "real" && k.gameData.director === "real",
        run: "generate_gamedata",
        folder: "game_data",
      },
      {
        key: "export_gamedata",
        done: () => false,
        run: "export_gamedata",
        folder: "gamedata_packs",
      },
      {
        key: "import_gamedata",
        done: (k) => k.gameData.backend === "real",
        run: "import_gamedata",
      },
      {
        key: "synthetic_data",
        done: (k) => k.gameData.backend !== "missing",
        run: "synthetic_data",
      },
    ],
  },
];

export default function BuildPage() {
  const t = useTranslations("build");
  const { api, kit } = useBridge();
  const act = useAct();
  const job = kit?.job;
  const running = job?.state === "running";

  const runStep = (s: Step) =>
    act(() =>
      s.run === "choose_dumper"
        ? api!.hostChooseDumper()
        : s.run === "import_sdk_folder"
          ? api!.hostImportSdkFolder()
          : s.run === "import_gamedata"
            ? api!.hostImportGameData()
            : api!.hostAction(s.run),
    );

  let banner: ReactNode = null;
  if (job) {
    banner = (
      <Banner
        status={job.state === "failed" ? "error" : job.state === "done" ? "success" : "info"}
        title={t(`jobStates.${job.state}`, { job: t(`jobs.${job.kind}`) })}
        description={job.message ?? undefined}
        collapsible={false}
        endContent={
          job.logFile ? (
            <Button
              label={t("openLog")}
              size="sm"
              clickAction={async () => void (await act(() => api!.hostOpen("host_logs")))}
            />
          ) : undefined
        }
      />
    );
  }

  return (
    <ManagerShell title={t("title")} description={t("description")}>
      {banner}
      {kit && !kit.toolchain.msbuild && (
        <Banner status="warning" title={t("noMsbuild")} collapsible={false} />
      )}
      {STAGES.map((stage) => (
        <Card key={stage.key} padding={0}>
          <Stack paddingInline={4} paddingBlock={3} gap={1}>
            <Heading level={2}>{t(`stages.${stage.key}`)}</Heading>
            <Text type="supporting">{t(`stageHints.${stage.key}`)}</Text>
          </Stack>
          <List hasDividers>
            {stage.steps.map((s) => {
              const done = kit ? s.done(kit) : false;
              return (
                <ListItem
                  key={s.key}
                  label={t(`steps.${s.key as "dumper"}.title`)}
                  description={t(`steps.${s.key as "dumper"}.hint`)}
                  startContent={
                    <StatusDot
                      variant={done ? "success" : "neutral"}
                      label={done ? t("done") : t("notDone")}
                    />
                  }
                  endContent={
                    <Stack direction="horizontal" gap={1}>
                      {s.folder && (
                        <Button
                          label={t("open")}
                          size="sm"
                          variant="ghost"
                          icon={<FolderOpen size={14} />}
                          isIconOnly
                          clickAction={async () => void (await act(() => api!.hostOpen(s.folder!)))}
                        />
                      )}
                      <Button
                        label={done ? t("again") : t("run")}
                        size="sm"
                        variant={done ? "ghost" : "secondary"}
                        isDisabled={running || !kit?.workspaceOk}
                        clickAction={async () => void (await runStep(s))}
                      />
                    </Stack>
                  }
                />
              );
            })}
          </List>
        </Card>
      ))}
    </ManagerShell>
  );
}

"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { MetadataList, MetadataListItem } from "@astryxdesign/core/MetadataList";
import { ProgressBar } from "@astryxdesign/core/ProgressBar";
import { Spinner } from "@astryxdesign/core/Spinner";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { TextArea } from "@astryxdesign/core/TextArea";
import { TextInput } from "@astryxdesign/core/TextInput";
import type { ActionResult, Snapshot, TaskProgress } from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { Download, FolderOpen, Play, ShieldCheck } from "lucide-react";
import { useRouter } from "next/navigation";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";

/** Runs a bridge action and toasts its refusal in the catalog's words; the snapshot carries the rest. */
function useAct() {
  const te = useTranslations("errors");
  return async (fn: () => Promise<ActionResult>) => {
    const r = await fn();
    if (!r.ok) toast.error(te(r.error.code, { detail: r.error.detail ?? "" }));
    return r.ok;
  };
}

function StepFrame({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <Stack gap={4}>
      <Stack gap={1}>
        <Heading level={2}>{title}</Heading>
        {description && <Text type="supporting">{description}</Text>}
      </Stack>
      {children}
    </Stack>
  );
}

export function JoinStep() {
  const t = useTranslations("launcher.join");
  const { api } = useBridge();
  const act = useAct();
  const [invite, setInvite] = useState("");
  const [busy, setBusy] = useState(false);
  const [changed, setChanged] = useState<string | null>(null);

  async function join(acceptNewCertificate = false) {
    if (!api) return;
    setBusy(true);
    const check = await api.checkInvite(invite.trim());
    if (!check.ok) {
      setBusy(false);
      return void act(async () => check);
    }
    if (check.certificateChanged && !acceptNewCertificate) {
      setChanged(check.previousFingerprint);
      setBusy(false);
      return;
    }
    await act(() =>
      api.submitInvite(invite.trim(), acceptNewCertificate, Boolean(check.movedFrom)),
    );
    setBusy(false);
  }

  return (
    <StepFrame title={t("title")} description={t("description")}>
      <TextArea
        label={t("invite")}
        value={invite}
        onChange={setInvite}
        rows={3}
        placeholder={t("placeholder")}
        hasSpellCheck={false}
      />
      {changed !== null && (
        <Banner
          status="warning"
          title={t("certChanged")}
          description={t("certChangedHint")}
          collapsible={false}
          endContent={
            <Button
              label={t("trustNew")}
              size="sm"
              variant="destructive"
              onClick={() => void join(true)}
            />
          }
        />
      )}
      <Stack direction="horizontal" justify="end">
        <Button
          label={t("join")}
          variant="primary"
          size="lg"
          isDisabled={!invite.trim()}
          isLoading={busy}
          onClick={() => void join()}
        />
      </Stack>
    </StepFrame>
  );
}

export function ConnectStep({ snapshot }: { snapshot: Snapshot }) {
  const t = useTranslations("launcher.connect");
  const { api } = useBridge();
  const act = useAct();
  const problem = snapshot.connect.problem;
  return (
    <StepFrame
      title={problem ? t(`problems.${problem}`) : t("title")}
      description={problem ? t(`hints.${problem}`) : t("description")}
    >
      {!problem && <Spinner size="lg" />}
      {problem && (
        <Stack direction="horizontal" gap={2}>
          <Button
            label={t("retry")}
            variant="primary"
            isLoading={snapshot.connect.checking}
            clickAction={async () => void (await act(() => api!.retryConnect()))}
          />
          {problem === "tailscale_missing" && (
            <Button
              label={t("getTailscale")}
              clickAction={async () =>
                void (await act(() => api!.openExternal("tailscale_download")))
              }
            />
          )}
          <Button
            label={t("forget")}
            variant="ghost"
            clickAction={async () => void (await act(() => api!.forgetServer()))}
          />
        </Stack>
      )}
    </StepFrame>
  );
}

export function RegisterStep() {
  const t = useTranslations("launcher.register");
  const { api } = useBridge();
  const act = useAct();
  const [name, setName] = useState("");
  const [key, setKey] = useState("");
  const [haveKey, setHaveKey] = useState(false);
  const valid = /^\w{3,16}$/.test(name);
  const register = () =>
    act(async () => {
      const r = await api!.register(name);
      return r.ok ? { ok: true } : r;
    });
  return (
    <StepFrame title={t("title")} description={t("description")}>
      {haveKey ? (
        <Stack gap={3}>
          <TextArea
            label={t("key")}
            description={t("keyHint")}
            value={key}
            onChange={setKey}
            rows={2}
            hasSpellCheck={false}
          />
          <Stack direction="horizontal" gap={2} justify="between">
            <Button label={t("newAccount")} variant="ghost" onClick={() => setHaveKey(false)} />
            <Stack direction="horizontal" gap={2}>
              <Button
                label={t("importFile")}
                icon={<FolderOpen size={16} />}
                clickAction={async () => void (await act(() => api!.importKeyFile()))}
              />
              <Button
                label={t("useKey")}
                variant="primary"
                isDisabled={!key.trim()}
                // biome-ignore lint/correctness/useHookAtTopLevel: a bridge method named use…, not a hook.
                clickAction={async () => void (await act(() => api!.useExistingKey(key.trim())))}
              />
            </Stack>
          </Stack>
        </Stack>
      ) : (
        <Stack gap={3}>
          <TextInput
            label={t("name")}
            description={t("nameHint")}
            value={name}
            onChange={setName}
            hasAutoFocus
            status={name && !valid ? { type: "error", message: t("nameInvalid") } : undefined}
            onEnter={() => valid && void register()}
          />
          <Stack direction="horizontal" gap={2} justify="between">
            <Button label={t("haveKey")} variant="ghost" onClick={() => setHaveKey(true)} />
            <Button
              label={t("create")}
              variant="primary"
              isDisabled={!valid}
              clickAction={async () => void (await register())}
            />
          </Stack>
        </Stack>
      )}
    </StepFrame>
  );
}

export function LocateStep({ snapshot }: { snapshot: Snapshot }) {
  const t = useTranslations("launcher.locate");
  const ti = useTranslations("launcher.install");
  const router = useRouter();
  const { api } = useBridge();
  const act = useAct();
  const { game } = snapshot;
  const hide = snapshot.settings.streamerMode;
  return (
    <StepFrame title={t("title")} description={t("description")}>
      {game.problem && game.problem !== "not_set" && (
        <Banner status="warning" title={t(`problems.${game.problem}`)} collapsible={false} />
      )}
      {game.detected.length > 0 && (
        <List hasDividers header={<Text type="label">{t("found")}</Text>}>
          {game.detected.map((d, i) => (
            <ListItem
              key={d.root}
              label={hide ? t("hiddenPath") : d.root}
              description={t("source", {
                source: t(`sources.${d.source}`),
                version: d.version ?? "?",
              })}
              endContent={
                <Button
                  label={t("use")}
                  size="sm"
                  variant="primary"
                  // biome-ignore lint/correctness/useHookAtTopLevel: a bridge method named use…, not a hook.
                  clickAction={async () => void (await act(() => api!.useDetectedGame(i)))}
                />
              }
            />
          ))}
        </List>
      )}
      <Stack direction="horizontal" gap={2} justify="end">
        <Button
          label={ti("fromEpic")}
          icon={<Download size={16} />}
          onClick={() => router.push("/game/")}
        />
        <Button
          label={t("choose")}
          icon={<FolderOpen size={16} />}
          clickAction={async () => void (await act(() => api!.chooseGameFolder()))}
        />
      </Stack>
    </StepFrame>
  );
}

export function RuntimeStep({ snapshot }: { snapshot: Snapshot }) {
  const t = useTranslations("launcher.runtime");
  const format = useFormatter();
  const { api } = useBridge();
  const act = useAct();
  const { runtime } = snapshot;
  return (
    <StepFrame
      title={t(`titles.${runtime.state}`)}
      description={t(`hints.${runtime.state}`, { version: runtime.version ?? "?" })}
    >
      {runtime.state === "untrusted" && runtime.files.length > 0 && (
        <MetadataList label={{ position: "start", width: 200 }}>
          {runtime.files.map((f) => (
            <MetadataListItem key={f.name} label={f.name}>
              <Text type="code" maxLines={1}>
                {t("fileSize", { kb: format.number(f.size / 1024, { maximumFractionDigits: 0 }) })}{" "}
                · {f.sha256.slice(0, 16)}…
              </Text>
            </MetadataListItem>
          ))}
        </MetadataList>
      )}
      <Stack direction="horizontal" gap={2} justify="end">
        {runtime.state === "untrusted" && (
          <Button
            label={t("trust")}
            icon={<ShieldCheck size={16} />}
            clickAction={async () => void (await act(() => api!.trustRuntime()))}
          />
        )}
        {runtime.state !== "unavailable" && runtime.state !== "untrusted" && (
          <Button
            label={t("install")}
            variant="primary"
            clickAction={async () => void (await act(() => api!.installRuntime()))}
          />
        )}
      </Stack>
    </StepFrame>
  );
}

export function TaskStep({ task }: { task: TaskProgress | null }) {
  const t = useTranslations("launcher.task");
  const format = useFormatter();
  const { api } = useBridge();
  if (!task) return <Spinner size="lg" />;
  const pct = task.totalBytes > 0 ? (task.doneBytes / task.totalBytes) * 100 : 0;
  return (
    <StepFrame title={t(`kinds.${task.kind}`)} description={task.currentFile ?? undefined}>
      <ProgressBar label={t(`kinds.${task.kind}`)} isLabelHidden value={pct} hasValueLabel />
      <Stack direction="horizontal" justify="between" align="center">
        <Text type="supporting">
          {t("files", { done: task.filesDone, total: task.filesTotal })} ·{" "}
          {format.number(task.bytesPerSecond / 1_000_000, { maximumFractionDigits: 1 })} MB/s
          {task.etaSeconds !== null && ` · ${t("eta", { seconds: task.etaSeconds })}`}
        </Text>
        <Button
          label={t("cancel")}
          variant="ghost"
          size="sm"
          onClick={() => void api?.cancelTask()}
        />
      </Stack>
    </StepFrame>
  );
}

export function ReadyStep({ snapshot }: { snapshot: Snapshot }) {
  const t = useTranslations("launcher.ready");
  const tq = useTranslations("launcher.queue");
  const { api } = useBridge();
  const act = useAct();
  const running = snapshot.phase === "running";
  // PLAY is waiting in the server's login queue
  const place = running ? null : snapshot.queuePosition;
  return (
    <Stack gap={4}>
      {snapshot.account.offerBackup && (
        <Banner
          status="warning"
          title={t("backupTitle")}
          description={t("backupHint")}
          collapsible={false}
          isDismissable
          onDismiss={() => void api?.dismissBackupOffer()}
          endContent={
            <Button
              label={t("backup")}
              size="sm"
              clickAction={async () => void (await act(() => api!.saveKeyBackup()))}
            />
          }
        />
      )}
      <Stack direction="horizontal" gap={3} align="center" justify="between" wrap="wrap">
        <Stack gap={1}>
          <Heading level={2}>
            {place !== null
              ? tq("position", { position: place })
              : running
                ? t("running")
                : t("title")}
          </Heading>
          <Text type="supporting">
            {place !== null
              ? tq("hint")
              : running
                ? t("runningHint", { port: snapshot.running.relayPort ?? 0 })
                : t("description")}
          </Text>
        </Stack>
        {place !== null ? (
          <Button
            label={tq("leave")}
            variant="secondary"
            size="lg"
            width={220}
            onClick={() => void api?.leaveQueue()}
          />
        ) : (
          <Button
            label={running ? t("playing") : t("play")}
            variant="primary"
            size="lg"
            width={220}
            icon={<Play size={18} />}
            isDisabled={running || snapshot.busy}
            clickAction={async () => void (await act(() => api!.play()))}
          />
        )}
      </Stack>
    </Stack>
  );
}

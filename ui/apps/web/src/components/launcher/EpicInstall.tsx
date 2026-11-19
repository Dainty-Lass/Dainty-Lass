"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Heading } from "@astryxdesign/core/Heading";
import { ProgressBar } from "@astryxdesign/core/ProgressBar";
import { Spinner } from "@astryxdesign/core/Spinner";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { TextArea } from "@astryxdesign/core/TextArea";
import { Token } from "@astryxdesign/core/Token";
import type {
  ActionResult,
  EpicLibraryCheck,
  EpicView,
  GameVersionEntry,
  GameVersionList,
  InstallInfo,
  InstallState,
  LauncherApi,
  Snapshot,
  TaskProgress,
} from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { FolderOpen } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { UninstallGame } from "./UninstallGame";

const ACCOUNT_POLL_MS = 1500;
const INSTALL_POLL_MS = 1000;

/** epic::auth::LOGIN_URL: shown only for the paste fallback, since the sign-in window opens it itself. */
const EPIC_LOGIN_URL =
  "https://www.epicgames.com/id/login?redirectUrl=https%3A%2F%2Fwww.epicgames.com%2Fid%2Fapi%2Fredirect%3FclientId%3D34a02cf8f4414e29b15921876da36f9a%26responseType%3Dcode";

// A paused install still holds the backend's one install slot
const RUNNING: readonly InstallState[] = ["preparing", "downloading", "paused", "finishing"];
export const isRunning = (state: InstallState) => RUNNING.includes(state);

/** Backslashes, a trailing slash and a drive letter's case don't tell two spellings of a folder apart. */
export function sameFolder(a: string | null, b: string | null): boolean {
  if (a === null || b === null) return false;
  const norm = (path: string) => {
    const slashes = path.replace(/\\/g, "/").replace(/\/+$/, "");
    return /^[A-Za-z]:/.test(slashes) ? slashes.toLowerCase() : slashes;
  };
  return norm(a) === norm(b);
}

/**
 * The channel's progress is newer than the polled one but names no version, so it counts only for the same
 * download (same total) as the one polled for.
 */
export function installProgress(
  info: InstallInfo | null,
  channel: TaskProgress | null,
): TaskProgress | null {
  if (!info || !isRunning(info.state)) return null;
  if (
    channel?.kind === "install" &&
    (info.progress === null || channel.totalBytes === info.progress.totalBytes)
  )
    return channel;
  return info.progress;
}

type Action = "install" | "continue" | "retry" | "pause" | "resume" | "cancel" | "discard" | "use";
type Hint = "signInFirst" | "notOwned" | "otherRunning" | null;

/** The buttons one version offers, and why an install button is off. */
export function versionActions(
  entry: GameVersionEntry,
  info: InstallInfo | null,
  ctx: { signedIn: boolean; owned: boolean | null; otherRunning: boolean; inUse: boolean },
): { actions: Action[]; hint: Hint } {
  if (entry.install === "locate" || entry.epic === null) return { actions: [], hint: null };
  const state = info?.state ?? "not_installed";
  if (state === "installed") return { actions: ctx.inUse ? [] : ["use"], hint: null };
  if (state === "finishing") return { actions: [], hint: null };
  if (state === "paused") return { actions: ["resume", "cancel", "discard"], hint: null };
  if (isRunning(state)) return { actions: ["pause", "cancel", "discard"], hint: null };
  if (entry.epic.manifest !== "ready" && state !== "resumable") return { actions: [], hint: null };
  const action: Action =
    state === "resumable" ? "continue" : state === "failed" ? "retry" : "install";
  const hint: Hint = !ctx.signedIn
    ? "signInFirst"
    : ctx.owned === false
      ? "notOwned"
      : ctx.otherRunning
        ? "otherRunning"
        : null;
  // A download stopped part-way can be thrown away instead of continued
  const discard: Action[] = state === "resumable" || state === "failed" ? ["discard"] : [];
  return { actions: [action, ...discard], hint };
}

/** Runs a bridge action and toasts its refusal in the catalog's words. */
function useReport() {
  const te = useTranslations("errors");
  return useCallback(
    (r: ActionResult) => {
      if (!r.ok) toast.error(te(r.error.code, { detail: r.error.detail ?? "" }));
      return r.ok;
    },
    [te],
  );
}

/** The Epic account, the library check, the versions and each install, over the epic_* and game_* commands. */
function useEpic(api: LauncherApi | null) {
  const report = useReport();
  const [account, setAccount] = useState<EpicView | null>(null);
  const [library, setLibrary] = useState<EpicLibraryCheck | null>(null);
  const [list, setList] = useState<GameVersionList | null>(null);
  const [installs, setInstalls] = useState<Record<string, InstallInfo>>({});
  const [working, setWorking] = useState<string | null>(null);
  const installsRef = useRef(installs);
  installsRef.current = installs;

  const refreshAccount = useCallback(async () => {
    if (!api) return;
    try {
      setAccount(await api.epicGet());
    } catch (error) {
      console.error(error);
    }
  }, [api]);

  const refreshInstall = useCallback(
    async (version: string) => {
      if (!api) return;
      const outcome = await api.gameInstallGet(version);
      if (outcome.ok) setInstalls((prev) => ({ ...prev, [version]: outcome.install }));
    },
    [api],
  );

  // Each Epic version's manifest is fetched for its sizes, so this takes a moment
  const loadList = useCallback(async () => {
    if (!api) return;
    const next = await api.gameVersionList();
    setList(next);
    if (!next.ok) return;
    const outcomes = await Promise.all(
      next.versions.filter((v) => v.install === "epic").map((v) => api.gameInstallGet(v.version)),
    );
    setInstalls((prev) => {
      const out = { ...prev };
      for (const o of outcomes) if (o.ok) out[o.install.version] = o.install;
      return out;
    });
  }, [api]);

  useEffect(() => {
    void refreshAccount();
    void loadList();
  }, [refreshAccount, loadList]);

  // The backend keeps Epic's answer for a day
  const signedIn = account?.signedIn === true;
  useEffect(() => {
    if (!api || !signedIn) return setLibrary(null);
    let current = true;
    void api.epicLibraryCheck().then((r) => current && setLibrary(r));
    return () => {
      current = false;
    };
  }, [api, signedIn]);

  // The sign-in window runs apart from the page
  const signingIn = account?.signingIn === true;
  useEffect(() => {
    if (!signingIn) return;
    const timer = setInterval(() => void refreshAccount(), ACCOUNT_POLL_MS);
    return () => clearInterval(timer);
  }, [signingIn, refreshAccount]);

  const anyRunning = Object.values(installs).some((i) => isRunning(i.state));
  useEffect(() => {
    if (!anyRunning) return;
    const timer = setInterval(() => {
      for (const i of Object.values(installsRef.current))
        if (isRunning(i.state)) void refreshInstall(i.version);
    }, INSTALL_POLL_MS);
    return () => clearInterval(timer);
  }, [anyRunning, refreshInstall]);

  const guard = useCallback(async (what: string, run: () => Promise<void>) => {
    setWorking(what);
    try {
      await run();
    } finally {
      setWorking(null);
    }
  }, []);

  const signIn = () =>
    guard("sign_in", async () => {
      // Answers once the window closes
      const r = await api!.epicSignInStart();
      await refreshAccount();
      report(r);
    });

  const submitCode = async (code: string) => {
    let ok = false;
    await guard("code", async () => {
      ok = report(await api!.epicSignInCode(code));
      await refreshAccount();
    });
    return ok;
  };

  const signOut = () =>
    guard("sign_out", async () => {
      report(await api!.epicSignOut());
      await refreshAccount();
    });

  const act = (version: string, action: Action) =>
    guard(version, async () => {
      switch (action) {
        case "install":
        case "continue":
        case "retry": {
          const outcome = await api!.gameInstallStart(version);
          if (outcome.ok) setInstalls((prev) => ({ ...prev, [version]: outcome.install }));
          else report(outcome);
          break;
        }
        case "pause":
          report(await api!.gameInstallPause(version));
          break;
        case "resume":
          report(await api!.gameInstallResume(version));
          break;
        case "cancel":
          report(await api!.gameInstallCancel(version));
          break;
        case "discard":
          report(await api!.gameInstallDiscard(version));
          break;
        case "use": {
          const folder = installsRef.current[version]?.folder;
          // biome-ignore lint/correctness/useHookAtTopLevel: a bridge method named use…, not a hook.
          if (folder) report(await api!.useGamePath(folder));
          return;
        }
      }
      await refreshInstall(version);
    });

  // Every version's folder moves with it, so the list is read again
  const chooseFolder = () =>
    guard("folder", async () => {
      const r = await api!.gameInstallChooseFolder();
      if (r.ok || r.error.code !== "cancelled") report(r);
      if (r.ok) await loadList();
    });

  return {
    account,
    library: signedIn ? library : null,
    chooseFolder,
    list,
    installs,
    working,
    signIn,
    submitCode,
    signOut,
    act,
    reload: () => guard("list", loadList),
  };
}

function AccountCard({ epic }: { epic: ReturnType<typeof useEpic> }) {
  const t = useTranslations("launcher.install");
  const te = useTranslations("errors");
  const { api } = useBridge();
  const [pasting, setPasting] = useState(false);
  const [code, setCode] = useState("");
  const { account, library } = epic;

  if (!account) return <Spinner size="lg" />;

  return (
    <Card padding={5} elevation="low">
      <Stack gap={4}>
        <Stack gap={1}>
          <Heading level={2}>{t("accountTitle")}</Heading>
          <Text type="supporting">{t("accountDescription")}</Text>
        </Stack>
        {account.signedIn ? (
          <Stack gap={3}>
            <Stack direction="horizontal" gap={2} align="center" wrap="wrap">
              <Text weight="semibold">
                {account.displayName
                  ? t("signedInAs", { name: account.displayName })
                  : t("signedIn")}
              </Text>
              {library?.ok && (
                <Token
                  size="sm"
                  label={library.owned ? t("owned") : t("notOwned")}
                  color={library.owned ? "green" : "orange"}
                />
              )}
            </Stack>
            {library && !library.ok && (
              <Banner
                status="warning"
                title={te(library.error.code, { detail: library.error.detail ?? "" })}
                collapsible={false}
              />
            )}
            {library?.ok && !library.owned && (
              <Banner status="warning" title={t("notOwnedHint")} collapsible={false} />
            )}
            <Stack direction="horizontal" justify="end">
              <Button
                label={t("signOut")}
                variant="ghost"
                isLoading={epic.working === "sign_out"}
                onClick={() => void epic.signOut()}
              />
            </Stack>
          </Stack>
        ) : pasting ? (
          <Stack gap={3}>
            <Text type="supporting">{t("pasteHint")}</Text>
            <Stack direction="horizontal" gap={2} align="center" wrap="wrap">
              <Text type="supporting">{EPIC_LOGIN_URL.slice(0, 48)}…</Text>
              <Button
                label={t("copyLink")}
                size="sm"
                onClick={async () => {
                  if (await api?.copyText(EPIC_LOGIN_URL)) toast.success(t("copied"));
                }}
              />
            </Stack>
            <TextArea
              label={t("codeLabel")}
              value={code}
              onChange={setCode}
              rows={3}
              hasSpellCheck={false}
            />
            <Stack direction="horizontal" gap={2} justify="between">
              <Button label={t("back")} variant="ghost" onClick={() => setPasting(false)} />
              <Button
                label={t("submitCode")}
                variant="primary"
                isDisabled={!code.trim()}
                isLoading={epic.working === "code"}
                onClick={async () => {
                  if (await epic.submitCode(code.trim())) {
                    setCode("");
                    setPasting(false);
                  }
                }}
              />
            </Stack>
          </Stack>
        ) : (
          <Stack direction="horizontal" gap={2} justify="end" wrap="wrap">
            <Button label={t("pasteInstead")} variant="ghost" onClick={() => setPasting(true)} />
            <Button
              label={account.signingIn ? t("signingIn") : t("signIn")}
              variant="primary"
              size="lg"
              isLoading={account.signingIn || epic.working === "sign_in"}
              onClick={() => void epic.signIn()}
            />
          </Stack>
        )}
      </Stack>
    </Card>
  );
}

function VersionCard({
  entry,
  epic,
  snapshot,
}: {
  entry: GameVersionEntry;
  epic: ReturnType<typeof useEpic>;
  snapshot: Snapshot;
}) {
  const t = useTranslations("launcher.install");
  const te = useTranslations("errors");
  const tt = useTranslations("launcher.task");
  const format = useFormatter();
  const { progress: channel } = useBridge();
  const info = epic.installs[entry.version] ?? null;
  const state = info?.state ?? "not_installed";
  const otherRunning = Object.values(epic.installs).some(
    (i) => i.version !== entry.version && isRunning(i.state),
  );
  const { actions, hint } = versionActions(entry, info, {
    signedIn: epic.account?.signedIn === true,
    owned: epic.library?.ok ? epic.library.owned : null,
    otherRunning,
    inUse: sameFolder(info?.folder ?? null, snapshot.game.root),
  });
  const progress = installProgress(info, channel);
  const [confirming, setConfirming] = useState(false);
  const gb = (bytes: number) => format.number(bytes / 1e9, { maximumFractionDigits: 1 });
  const hide = snapshot.settings.streamerMode;
  const manifest = entry.epic?.manifest ?? "ready";
  // A stopped install continues in the folder it began in, so only a fresh start can move
  const movable =
    (state === "not_installed" || state === "failed") &&
    !Object.values(epic.installs).some((i) => isRunning(i.state) || i.state === "resumable");

  return (
    <Card padding={5}>
      <Stack gap={3}>
        <Stack direction="horizontal" gap={2} align="center" justify="between" wrap="wrap">
          <Stack gap={0.5}>
            <Heading level={3}>{entry.displayName}</Heading>
            {entry.epic && (
              <Text type="supporting">{t("build", { build: entry.epic.buildVersion })}</Text>
            )}
          </Stack>
          <Token
            size="sm"
            label={t(`states.${state}`)}
            color={state === "installed" ? "green" : state === "failed" ? "red" : undefined}
          />
        </Stack>
        {entry.install === "locate" && <Text type="supporting">{t("locateOnly")}</Text>}
        {entry.epic?.downloadBytes != null && entry.epic.installBytes != null && (
          <Text type="supporting">
            {t("sizes", {
              download: gb(entry.epic.downloadBytes),
              install: gb(entry.epic.installBytes),
            })}
          </Text>
        )}
        {manifest !== "ready" && state !== "resumable" && entry.install === "epic" && (
          <Banner status="warning" title={t(`manifest.${manifest}`)} collapsible={false} />
        )}
        {entry.install === "epic" && (info?.folder || movable) && (
          <Stack direction="horizontal" gap={2} align="center" justify="between" wrap="wrap">
            {info?.folder && (
              <Text type="supporting">
                {t("folder", { folder: hide ? t("folderHidden") : info.folder })}
              </Text>
            )}
            {movable && (
              <Button
                label={t("changeFolder")}
                size="sm"
                variant="ghost"
                icon={<FolderOpen size={16} />}
                isLoading={epic.working === "folder"}
                onClick={() => void epic.chooseFolder()}
              />
            )}
          </Stack>
        )}
        {progress && (
          <Stack gap={1}>
            <ProgressBar
              label={t(`states.${state}`)}
              isLabelHidden
              value={progress.totalBytes > 0 ? (progress.doneBytes / progress.totalBytes) * 100 : 0}
              hasValueLabel
            />
            <Text type="supporting">
              {t("progress", { done: gb(progress.doneBytes), total: gb(progress.totalBytes) })} ·{" "}
              {format.number(progress.bytesPerSecond / 1_000_000, { maximumFractionDigits: 1 })}{" "}
              MB/s
              {progress.etaSeconds !== null && ` · ${tt("eta", { seconds: progress.etaSeconds })}`}
            </Text>
          </Stack>
        )}
        {info?.error && !isRunning(state) && (
          <Banner
            status="error"
            title={te(info.error.code, { detail: info.error.detail ?? "" })}
            collapsible={false}
          />
        )}
        {state === "installed" && actions.length === 0 && (
          <Text type="supporting">{t("inUse")}</Text>
        )}
        {state === "installed" && (
          <Stack direction="horizontal" justify="end">
            <UninstallGame
              version={entry.version}
              name={entry.displayName}
              hideFolder={hide ? t("folderHidden") : null}
              onDone={() => void epic.reload()}
            />
          </Stack>
        )}
        {confirming && actions.includes("discard") && (
          <Banner
            status="warning"
            title={t("discardTitle")}
            description={t("discardHint")}
            collapsible={false}
            endContent={
              <Stack direction="horizontal" gap={2}>
                <Button
                  label={t("discardKeep")}
                  size="sm"
                  variant="ghost"
                  onClick={() => setConfirming(false)}
                />
                <Button
                  label={t("discardConfirm")}
                  size="sm"
                  variant="destructive"
                  isLoading={epic.working === entry.version}
                  onClick={async () => {
                    await epic.act(entry.version, "discard");
                    setConfirming(false);
                  }}
                />
              </Stack>
            }
          />
        )}
        {(actions.length > 0 || hint) && (
          <Stack direction="horizontal" gap={2} align="center" justify="end" wrap="wrap">
            {hint && <Text type="supporting">{t(`hints.${hint}`)}</Text>}
            {actions.map((action, i) => (
              <Button
                key={action}
                label={t(`actions.${action}`)}
                variant={i === 0 && action !== "cancel" ? "primary" : "ghost"}
                // Signing in matters only to download; discarding needs no account
                isDisabled={hint !== null && action !== "discard"}
                isLoading={epic.working === entry.version && i === 0}
                onClick={() =>
                  action === "discard" ? setConfirming(true) : void epic.act(entry.version, action)
                }
              />
            ))}
          </Stack>
        )}
      </Stack>
    </Card>
  );
}

/** Epic sign-in and the game download (DE2, DE5, DE7): the archived manifest, Epic's CDN, the player's account. */
export function EpicInstall({ snapshot }: { snapshot: Snapshot }) {
  const t = useTranslations("launcher.install");
  const te = useTranslations("errors");
  const { api } = useBridge();
  const epic = useEpic(api);
  const list = epic.list;

  return (
    <Stack gap={4}>
      <AccountCard epic={epic} />
      <Stack direction="horizontal" gap={2} align="center" justify="between">
        <Heading level={2}>{t("versionsTitle")}</Heading>
        <Button
          label={t("reload")}
          variant="ghost"
          size="sm"
          isLoading={epic.working === "list"}
          onClick={() => void epic.reload()}
        />
      </Stack>
      {list === null ? (
        <Spinner size="lg" />
      ) : !list.ok ? (
        <Banner
          status="error"
          title={te(list.error.code, { detail: list.error.detail ?? "" })}
          collapsible={false}
        />
      ) : (
        list.versions.map((entry) => (
          <VersionCard key={entry.version} entry={entry} epic={epic} snapshot={snapshot} />
        ))
      )}
    </Stack>
  );
}

"use client";

import { AlertDialog } from "@astryxdesign/core/AlertDialog";
import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import type { SavedServerInfo } from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { useRouter } from "next/navigation";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { LauncherShell } from "@/components/launcher/LauncherShell";

export default function Servers() {
  const t = useTranslations("launcher.servers");
  const te = useTranslations("errors");
  const router = useRouter();
  const { snapshot, api } = useBridge();
  const [removing, setRemoving] = useState<SavedServerInfo | null>(null);
  const hide = snapshot?.settings.streamerMode;

  const act = async (
    fn: () => Promise<{ ok: boolean; error?: { code: string; detail?: string } }>,
  ) => {
    const r = await fn();
    if (!r.ok && r.error)
      toast.error(te(r.error.code as "unknown", { detail: r.error.detail ?? "" }));
    return r.ok;
  };

  return (
    <LauncherShell>
      <Stack gap={4} padding={6} isScrollable height="100%">
        <Stack direction="horizontal" justify="between" align="end">
          <Stack gap={1}>
            <Heading level={1}>{t("title")}</Heading>
            <Text type="supporting">{t("description")}</Text>
          </Stack>
          <Button
            label={t("add")}
            variant="primary"
            clickAction={async () => {
              if (await act(() => api!.forgetServer())) router.push("/");
            }}
          />
        </Stack>
        <Card padding={0}>
          {!snapshot?.savedServers.length ? (
            <EmptyState title={t("empty")} description={t("emptyHint")} />
          ) : (
            <List hasDividers>
              {snapshot.savedServers.map((s) => (
                <ListItem
                  key={s.id}
                  label={s.name}
                  description={[
                    s.username ? t("as", { name: hide ? "•••" : s.username }) : t("noAccount"),
                    hide ? null : `${s.host}:${s.port}`,
                    t(`modes.${s.mode}`),
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                  startContent={
                    s.current ? (
                      <Badge variant="success" label={t("current")} />
                    ) : s.unsupported ? (
                      <Badge variant="neutral" label={t("unsupported")} />
                    ) : undefined
                  }
                  endContent={
                    <Stack direction="horizontal" gap={1}>
                      {!s.current && !s.unsupported && (
                        <Button
                          label={t("switch")}
                          size="sm"
                          clickAction={async () => {
                            if (await act(() => api!.switchServer(s.id))) router.push("/");
                          }}
                        />
                      )}
                      <Button
                        label={t("remove")}
                        size="sm"
                        variant="ghost"
                        onClick={() => setRemoving(s)}
                      />
                    </Stack>
                  }
                />
              ))}
            </List>
          )}
        </Card>
      </Stack>
      <AlertDialog
        isOpen={removing !== null}
        onOpenChange={(open) => !open && setRemoving(null)}
        title={t("removeTitle", { name: removing?.name ?? "" })}
        description={t("removeBody")}
        actionLabel={t("remove")}
        onAction={async () => {
          if (removing && (await act(() => api!.removeSavedServer(removing.id)))) setRemoving(null);
        }}
      />
    </LauncherShell>
  );
}

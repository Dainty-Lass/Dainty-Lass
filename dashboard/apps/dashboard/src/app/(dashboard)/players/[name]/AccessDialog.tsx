"use client";

import { Button } from "@astryxdesign/core/Button";
import { CheckboxInput } from "@astryxdesign/core/CheckboxInput";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { Selector } from "@astryxdesign/core/Selector";
import { Stack } from "@astryxdesign/core/Stack";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { ReasonField } from "@/components/ReasonField";
import { useAction } from "@/components/useAction";
import type { BackendAccount } from "@/lib/game/backend";
import { accessAction, rolesAction } from "../actions";

// The backend lets these be toggled from its admin routes (security/testerFeatures.ts).
const ASSIGNABLE_ROLES = ["tester"];

/** The backend's own account state, separate from the game's bans: approval and disabling. */
export function AccessDialog({
  isOpen,
  controllerId,
  account,
  canRoles,
  reasons,
  onClose,
}: {
  isOpen: boolean;
  controllerId: string;
  account: BackendAccount;
  canRoles: boolean;
  reasons: string[];
  onClose: () => void;
}) {
  const t = useTranslations("players");
  const { run } = useAction();
  const [status, setStatus] = useState(account.status);
  const [approval, setApproval] = useState(account.approvalStatus);
  const [roles, setRoles] = useState(account.roles.filter((r) => ASSIGNABLE_ROLES.includes(r)));
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);

  const accessChanged = status !== account.status || approval !== account.approvalStatus;
  const current = account.roles.filter((r) => ASSIGNABLE_ROLES.includes(r));
  const rolesChanged = roles.slice().sort().join() !== current.slice().sort().join();

  async function save() {
    setBusy(true);
    let ok = true;
    if (accessChanged) {
      ok =
        (await run(
          () =>
            accessAction(
              controllerId,
              account.userId,
              status !== account.status ? status : undefined,
              approval !== account.approvalStatus ? approval : undefined,
              reason,
            ),
          t("accessSaved"),
        )) !== false;
    }
    if (ok && rolesChanged)
      ok =
        (await run(
          () => rolesAction(controllerId, account.userId, roles, reason),
          t("rolesSaved"),
        )) !== false;
    setBusy(false);
    if (ok) onClose();
  }

  return (
    <Dialog isOpen={isOpen} onOpenChange={(open) => !open && onClose()} purpose="form" width={480}>
      <Stack gap={4}>
        <DialogHeader
          title={t("accessTitle", { name: account.displayName })}
          subtitle={t("accessExplain")}
          onOpenChange={() => onClose()}
        />
        <Stack gap={4} paddingInline={4}>
          <Selector
            label={t("fields.access")}
            value={status}
            onChange={(v) => setStatus(v as typeof status)}
            options={(["active", "disabled", "banned"] as const).map((s) => ({
              value: s,
              label: t(`statuses.${s}`),
            }))}
            width="100%"
          />
          <Selector
            label={t("fields.approval")}
            value={approval}
            onChange={(v) => setApproval(v as typeof approval)}
            options={(["approved", "pending", "rejected"] as const).map((s) => ({
              value: s,
              label: t(`approval.${s}`),
            }))}
            width="100%"
          />
          {canRoles &&
            ASSIGNABLE_ROLES.map((role) => (
              <CheckboxInput
                key={role}
                label={t("roleLabel", { role })}
                description={t("roleTesterExplain")}
                value={roles.includes(role)}
                onChange={(on: boolean) =>
                  setRoles((prev) => (on ? [...prev, role] : prev.filter((r) => r !== role)))
                }
              />
            ))}
          <ReasonField
            label={t("changeReason")}
            presets={reasons}
            value={reason}
            onChange={setReason}
          />
        </Stack>
        <Stack direction="horizontal" gap={2} justify="end" paddingInline={4} paddingBlockEnd={4}>
          <Button label={t("cancel")} variant="ghost" onClick={onClose} />
          <Button
            label={t("saveAccess")}
            variant="primary"
            isDisabled={(!accessChanged && !rolesChanged) || reason.trim().length < 3}
            isLoading={busy}
            onClick={save}
          />
        </Stack>
      </Stack>
    </Dialog>
  );
}

"use client";

import { AppShell } from "@astryxdesign/core/AppShell";
import { Badge } from "@astryxdesign/core/Badge";
import { Heading } from "@astryxdesign/core/Heading";
import { SideNav, SideNavHeading, SideNavItem, SideNavSection } from "@astryxdesign/core/SideNav";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Text } from "@astryxdesign/core/Text";
import type { ActionResult, HostServerState } from "@dainty/bridge";
import { TitleBar, useBridge } from "@dainty/bridge/react";
import { BrandMark, ThemeModeControl } from "@dainty/theme";
import { Hammer, ListChecks, Server } from "lucide-react";
import { usePathname } from "next/navigation";
import { useTranslations } from "next-intl";
import type { ReactNode } from "react";
import { toast } from "sonner";

export const STATE_DOT: Record<
  HostServerState,
  "success" | "warning" | "error" | "neutral" | "accent"
> = {
  running: "success",
  starting: "warning",
  stopping: "warning",
  stopped: "neutral",
  external: "accent",
  failed: "error",
};

/** Runs a bridge action, toasting a refusal in the catalog's words. */
export function useAct() {
  const te = useTranslations("errors");
  return async (fn: () => Promise<ActionResult>, success?: string) => {
    const r = await fn();
    if (!r.ok) toast.error(te(r.error.code, { detail: r.error.detail ?? "" }));
    else if (success) toast.success(success);
    return r.ok;
  };
}

export function ManagerShell({
  title,
  description,
  actions,
  children,
}: {
  title: string;
  description?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}) {
  const t = useTranslations("shell");
  const tw = useTranslations("window");
  const tc = useTranslations("theme");
  const ts = useTranslations("server.states");
  const { kit, server } = useBridge();
  const path = usePathname();
  const at = (href: string) => (href === "/" ? path === "/" : path.startsWith(href));
  const missing = kit?.requirements.filter((r) => r.state !== "ok").length ?? 0;
  const name = kit?.serverConfig.name ?? t("product");

  return (
    <AppShell
      variant="wash"
      height="fill"
      topNav={
        <TitleBar
          title={t("windowTitle", { name })}
          start={<BrandMark size={18} />}
          labels={{ minimize: tw("minimize"), maximize: tw("maximize"), close: tw("close") }}
        />
      }
      sideNav={
        <SideNav
          collapsible
          header={
            <SideNavHeading
              heading={name}
              subheading={t("product")}
              icon={<BrandMark size={24} />}
            />
          }
          footer={
            <Stack gap={2} padding={2}>
              {server && (
                <Stack direction="horizontal" gap={2} align="center">
                  <StatusDot
                    variant={STATE_DOT[server.state]}
                    label={ts(server.state)}
                    isPulsing={server.state === "running"}
                  />
                  <Text type="supporting">{ts(server.state)}</Text>
                </Stack>
              )}
              <ThemeModeControl
                label={tc("label")}
                labels={{ system: tc("system"), light: tc("light"), dark: tc("dark") }}
              />
            </Stack>
          }
        >
          <SideNavSection title={t("navigation")} isHeaderHidden>
            <SideNavItem href="/" label={t("server")} icon={Server} isSelected={at("/")} />
            <SideNavItem
              href="/setup/"
              label={t("setup")}
              icon={ListChecks}
              isSelected={at("/setup")}
              endContent={missing ? <Badge variant="warning" label={String(missing)} /> : undefined}
            />
            <SideNavItem
              href="/build/"
              label={t("build")}
              icon={Hammer}
              isSelected={at("/build")}
              endContent={
                kit?.job?.state === "running" ? (
                  <StatusDot variant="warning" label={t("jobRunning")} isPulsing />
                ) : undefined
              }
            />
          </SideNavSection>
        </SideNav>
      }
    >
      <Stack gap={4} padding={6} isScrollable height="100%">
        <Stack direction="horizontal" gap={4} justify="between" align="end" wrap="wrap">
          <Stack gap={1}>
            <Heading level={1}>{title}</Heading>
            {description && <Text type="supporting">{description}</Text>}
          </Stack>
          {actions && (
            <Stack direction="horizontal" gap={2}>
              {actions}
            </Stack>
          )}
        </Stack>
        {children}
      </Stack>
    </AppShell>
  );
}

"use client";

import { AppShell } from "@astryxdesign/core/AppShell";
import { Avatar } from "@astryxdesign/core/Avatar";
import { Badge } from "@astryxdesign/core/Badge";
import { IconButton } from "@astryxdesign/core/IconButton";
import { Selector } from "@astryxdesign/core/Selector";
import { SideNav, SideNavHeading, SideNavItem, SideNavSection } from "@astryxdesign/core/SideNav";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { BrandMark, HUES, type Hue, RAIL_RESIZE, ThemeModeControl } from "@dainty/theme";
import {
  Bug,
  Flag,
  History,
  LayoutDashboard,
  LogOut,
  MessagesSquare,
  Server,
  Settings,
  ShieldBan,
  UserCog,
  UserRound,
  Users,
} from "lucide-react";
import { usePathname } from "next/navigation";
import { useTranslations } from "next-intl";
import type { ReactNode } from "react";
import { ControllerProvider } from "@/components/ControllerContext";
import { ControllerState } from "@/components/ControllerState";
import { useAction } from "@/components/useAction";
import { signOutAction } from "@/lib/auth/sign-out";
import type { ControllerState as State } from "@/lib/fleet/status";
import { setScopeAction } from "./controllers/actions";

export interface ShellProps {
  serverName: string;
  me: { name: string; role: string; avatar?: string };
  openReports: number;
  /** A controller id, or "all". */
  scope: string;
  controllers: { id: string; name: string; state: State }[];
  /** Accounts waiting on approval, for admins. */
  pending: number;
  nav: {
    reports: boolean;
    crashReports: boolean;
    chat: boolean;
    audit: boolean;
    controllers: boolean;
    staff: boolean;
    settings: boolean;
  };
  children: ReactNode;
}

export function DashboardShell({
  serverName,
  me,
  openReports,
  pending,
  scope,
  controllers,
  nav,
  children,
}: ShellProps) {
  const t = useTranslations("nav");
  const tc = useTranslations("common");
  const tf = useTranslations("fleet");
  const { run } = useAction();
  const path = usePathname();
  const down = controllers.filter((c) => c.state === "offline" || c.state === "stale").length;
  const at = (href: string) =>
    href === "/" ? path === "/" : path === href || path.startsWith(`${href}/`);

  // An element in its hue (Caddy Proxy Manager's rail): a component would be drawn smaller.
  const item = (href: string, label: string, Icon: typeof Users, hue: Hue, end?: ReactNode) => (
    <SideNavItem
      key={href}
      href={href}
      label={label}
      icon={<Icon className={HUES[hue].text} />}
      isSelected={at(href)}
      endContent={end}
    />
  );

  return (
    <AppShell
      variant="wash"
      sideNav={
        <SideNav
          collapsible
          resizable={RAIL_RESIZE}
          topContent={
            controllers.length > 1 ? (
              <Selector
                label={tf("scope")}
                isLabelHidden
                size="sm"
                width="100%"
                hasSearch={controllers.length > 8}
                value={scope}
                options={[
                  { value: "all", label: tf("allControllers", { count: controllers.length }) },
                  { type: "divider" as const },
                  ...controllers.map((c) => ({
                    value: c.id,
                    label: c.name,
                    endContent: <ControllerState state={c.state} withText={false} />,
                  })),
                ]}
                onChange={(v) => void run(() => setScopeAction(v))}
              />
            ) : undefined
          }
          header={
            <SideNavHeading
              heading={serverName}
              subheading={t("product")}
              headingHref="/"
              icon={<BrandMark size={24} />}
            />
          }
          footer={
            <Stack gap={2} paddingInline={2} paddingBlock={2}>
              <Stack direction="horizontal" gap={2} align="center">
                <Avatar name={me.name} src={me.avatar} size="sm" />
                <Stack gap={0} width="100%">
                  <Text weight="medium" maxLines={1}>
                    {me.name}
                  </Text>
                  <Text type="supporting">{tc(`roles.${me.role as "owner"}`)}</Text>
                </Stack>
                <IconButton
                  label={t("account")}
                  icon={<UserRound size={16} />}
                  variant="ghost"
                  size="sm"
                  href="/account"
                />
                <form action={signOutAction}>
                  <IconButton
                    label={t("signOut")}
                    icon={<LogOut size={16} />}
                    variant="ghost"
                    size="sm"
                    type="submit"
                  />
                </form>
              </Stack>
              <ThemeModeControl
                label={tc("theme.label")}
                labels={{
                  system: tc("theme.system"),
                  light: tc("theme.light"),
                  dark: tc("theme.dark"),
                }}
              />
            </Stack>
          }
        >
          <SideNavSection title={t("moderation")}>
            {item("/", t("overview"), LayoutDashboard, "blue")}
            {item("/players", t("players"), Users, "cyan")}
            {nav.reports &&
              item(
                "/reports",
                t("reports"),
                Flag,
                "red",
                openReports > 0 ? <Badge variant="error" label={String(openReports)} /> : undefined,
              )}
            {nav.crashReports && item("/crash-reports", t("crashReports"), Bug, "green")}
            {nav.chat && item("/chat", t("chat"), MessagesSquare, "purple")}
            {nav.chat && item("/chat-filter", t("chatFilter"), ShieldBan, "orange")}
            {nav.audit && item("/audit", t("audit"), History, "yellow")}
            {nav.controllers &&
              item(
                "/controllers",
                t("controllers"),
                Server,
                "teal",
                down > 0 ? <Badge variant="error" label={String(down)} /> : undefined,
              )}
          </SideNavSection>
          {(nav.staff || nav.settings) && (
            <SideNavSection title={t("administration")}>
              {nav.staff &&
                item(
                  "/members",
                  t("members"),
                  UserCog,
                  "pink",
                  pending > 0 ? <Badge variant="warning" label={String(pending)} /> : undefined,
                )}
              {nav.settings && item("/settings", t("settings"), Settings, "gray")}
            </SideNavSection>
          )}
        </SideNav>
      }
    >
      <ControllerProvider
        value={{
          names: Object.fromEntries(controllers.map((c) => [c.id, c.name])),
          fleetWide: scope === "all" && controllers.length > 1,
        }}
      >
        {children}
      </ControllerProvider>
    </AppShell>
  );
}

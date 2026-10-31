"use client";

import { AppShell } from "@astryxdesign/core/AppShell";
import { Avatar } from "@astryxdesign/core/Avatar";
import { Badge } from "@astryxdesign/core/Badge";
import { IconButton } from "@astryxdesign/core/IconButton";
import { SideNav, SideNavHeading, SideNavItem, SideNavSection } from "@astryxdesign/core/SideNav";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { BrandMark, ThemeModeControl } from "@dainty/theme";
import {
  Flag,
  History,
  LayoutDashboard,
  LogOut,
  MessagesSquare,
  Settings,
  ShieldBan,
  UserCog,
  Users,
} from "lucide-react";
import { usePathname } from "next/navigation";
import { useTranslations } from "next-intl";
import type { ReactNode } from "react";

export interface ShellProps {
  serverName: string;
  me: { name: string; role: string; avatar?: string };
  openReports: number;
  nav: { filter: boolean; staff: boolean; settings: boolean };
  children: ReactNode;
}

export function DashboardShell({ serverName, me, openReports, nav, children }: ShellProps) {
  const t = useTranslations("nav");
  const tc = useTranslations("common");
  const path = usePathname();
  const at = (href: string) =>
    href === "/" ? path === "/" : path === href || path.startsWith(`${href}/`);

  const item = (href: string, label: string, icon: typeof Users, end?: ReactNode) => (
    <SideNavItem
      key={href}
      href={href}
      label={label}
      icon={icon}
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
                <form method="post" action="/api/auth/logout">
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
            {item("/", t("overview"), LayoutDashboard)}
            {item("/players", t("players"), Users)}
            {item(
              "/reports",
              t("reports"),
              Flag,
              openReports > 0 ? <Badge variant="error" label={String(openReports)} /> : undefined,
            )}
            {item("/chat", t("chat"), MessagesSquare)}
            {nav.filter && item("/chat-filter", t("chatFilter"), ShieldBan)}
            {item("/audit", t("audit"), History)}
          </SideNavSection>
          {(nav.staff || nav.settings) && (
            <SideNavSection title={t("administration")}>
              {nav.staff && item("/staff", t("staff"), UserCog)}
              {nav.settings && item("/settings", t("settings"), Settings)}
            </SideNavSection>
          )}
        </SideNav>
      }
    >
      {children}
    </AppShell>
  );
}

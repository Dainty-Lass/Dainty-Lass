"use client";

import { AppShell } from "@astryxdesign/core/AppShell";
import { Avatar } from "@astryxdesign/core/Avatar";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { SideNav, SideNavHeading, SideNavItem, SideNavSection } from "@astryxdesign/core/SideNav";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { TitleBar, useBridge } from "@dainty/bridge/react";
import { BrandMark, HUES, RAIL_RESIZE } from "@dainty/theme";
import { Download, Gamepad2, ScrollText, Server, Settings, Wrench } from "lucide-react";
import { usePathname } from "next/navigation";
import { useTranslations } from "next-intl";
import { type ReactNode, useEffect } from "react";
import { toast } from "sonner";

/** The launcher's frame: its own title bar, the rail, and the backend's error and notice, wherever they arise. */
export function LauncherShell({ children }: { children: ReactNode }) {
  const t = useTranslations("launcher.shell");
  const tw = useTranslations("window");
  const te = useTranslations("errors");
  const tn = useTranslations("launcher.notices");
  const ti = useTranslations("launcher.install");
  const th = useTranslations("launcher.host");
  const { snapshot, api } = useBridge();
  const path = usePathname();
  const at = (href: string) => (href === "/" ? path === "/" : path.startsWith(href));

  useEffect(() => {
    if (!snapshot?.notice || !api) return;
    toast.success(tn(snapshot.notice));
    void api.dismissNotice();
  }, [snapshot?.notice, api, tn]);

  const error = snapshot?.lastError;
  const account = snapshot?.account.username;

  return (
    <AppShell
      variant="wash"
      height="fill"
      topNav={
        <TitleBar
          title={snapshot?.server?.name ?? t("product")}
          start={<BrandMark size={18} />}
          labels={{ minimize: tw("minimize"), maximize: tw("maximize"), close: tw("close") }}
        />
      }
      sideNav={
        <SideNav
          collapsible
          resizable={RAIL_RESIZE}
          header={
            <SideNavHeading
              heading={t("product")}
              subheading={snapshot?.app.version ? `v${snapshot.app.version}` : undefined}
              icon={<BrandMark size={24} />}
            />
          }
          footer={
            account ? (
              <Stack direction="horizontal" gap={2} align="center" padding={2}>
                <Avatar name={account} size="sm" />
                <Stack gap={0}>
                  <Text weight="medium">
                    {snapshot?.settings.streamerMode ? t("hidden") : account}
                  </Text>
                  <Text type="supporting">{snapshot?.server?.name}</Text>
                </Stack>
              </Stack>
            ) : undefined
          }
        >
          <SideNavSection title={t("navigation")} isHeaderHidden>
            <SideNavItem
              href="/"
              label={t("play")}
              icon={<Gamepad2 className={HUES.green.text} />}
              isSelected={at("/")}
            />
            <SideNavItem
              href="/game/"
              label={ti("nav")}
              icon={<Download className={HUES.blue.text} />}
              isSelected={at("/game")}
            />
            <SideNavItem
              href="/servers/"
              label={t("servers")}
              icon={<Server className={HUES.teal.text} />}
              isSelected={at("/servers")}
            />
            <SideNavItem
              href="/log/"
              label={t("gameLog")}
              icon={<ScrollText className={HUES.cyan.text} />}
              isSelected={at("/log")}
            />
            {snapshot?.app.hostToolsAvailable && snapshot.settings.hostTools && (
              <SideNavItem
                href="/host/"
                label={th("nav")}
                icon={<Wrench className={HUES.orange.text} />}
                isSelected={at("/host")}
              />
            )}
            <SideNavItem
              href="/settings/"
              label={t("settings")}
              icon={<Settings className={HUES.gray.text} />}
              isSelected={at("/settings")}
            />
          </SideNavSection>
        </SideNav>
      }
    >
      <Stack gap={0} height="100%">
        {snapshot?.app.updateReady && (
          <Banner
            status="info"
            container="section"
            title={t("updateReady")}
            collapsible={false}
            endContent={
              <Button
                label={t("restartToUpdate")}
                size="sm"
                variant="primary"
                onClick={() => void api?.installUpdate()}
              />
            }
          />
        )}
        {error && (
          <Banner
            status="error"
            container="section"
            title={te(error.code, { detail: error.detail ?? "" })}
            collapsible={false}
            isDismissable
            onDismiss={() => void api?.dismissError()}
          />
        )}
        {children}
      </Stack>
    </AppShell>
  );
}

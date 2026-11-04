"use client";

import { LinkProvider } from "@astryxdesign/core/Link";
import bridgeEn from "@dainty/bridge/messages/en.json";
import bridgePt from "@dainty/bridge/messages/pt-BR.json";
import { BridgeProvider, useBridge } from "@dainty/bridge/react";
import { type Accent, BootSplash, DaintyThemeProvider, parseAccent } from "@dainty/theme";
import NextLink from "next/link";
import { NextIntlClientProvider } from "next-intl";
import { type ReactNode, useEffect, useState } from "react";
import { Toaster } from "sonner";
import { MODE } from "@/mode";
import launcherEn from "../../../messages/launcher/en.json";
import launcherPt from "../../../messages/launcher/pt-BR.json";
import managerEn from "../../../messages/manager/en.json";
import managerPt from "../../../messages/manager/pt-BR.json";

const HOST = MODE === "manager";

// Each shell's catalog under its own namespace, beside the bridge's shared one.
const CATALOGS = {
  en: { ...bridgeEn, launcher: launcherEn, manager: managerEn },
  "pt-BR": { ...bridgePt, launcher: launcherPt, manager: managerPt },
} as const;

/** The language is the shell's own setting (Settings → Language), so the catalog follows the snapshot. */
function Localized({ children }: { children: ReactNode }) {
  const { snapshot, kit } = useBridge();
  const locale = snapshot?.settings.language ?? "en";
  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);
  // The manager's pages read the host kit, which the bridge loads after the snapshot
  const ready = snapshot !== null && (!HOST || kit !== null);
  return (
    <NextIntlClientProvider
      locale={locale}
      messages={CATALOGS[locale]}
      timeZone={Intl.DateTimeFormat().resolvedOptions().timeZone}
    >
      {children}
      <BootSplash ready={ready} />
    </NextIntlClientProvider>
  );
}

/** The server's branding may name an accent (the theme's own only), which "auto" follows. */
function Themed({ children }: { children: ReactNode }) {
  const { api } = useBridge();
  const [accent, setAccent] = useState<Accent | undefined>();
  useEffect(() => {
    if (HOST) return;
    void api?.getBranding().then((b) => setAccent(parseAccent(b.accent)));
  }, [api]);
  return (
    <DaintyThemeProvider persistence="local" accent={accent}>
      {children}
    </DaintyThemeProvider>
  );
}

export function DesktopProviders({ children }: { children: ReactNode }) {
  return (
    <BridgeProvider host={HOST}>
      <Themed>
        <Localized>
          <LinkProvider component={NextLink}>
            {children}
            <Toaster richColors position="bottom-right" />
          </LinkProvider>
        </Localized>
      </Themed>
    </BridgeProvider>
  );
}

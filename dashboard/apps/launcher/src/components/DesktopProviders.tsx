"use client";

import { LinkProvider } from "@astryxdesign/core/Link";
import bridgeEn from "@dainty/bridge/messages/en.json";
import bridgePt from "@dainty/bridge/messages/pt-BR.json";
import { BridgeProvider, useBridge } from "@dainty/bridge/react";
import { BootSplash, DaintyThemeProvider, parseAccent } from "@dainty/theme";
import NextLink from "next/link";
import { NextIntlClientProvider } from "next-intl";
import { type ReactNode, useEffect } from "react";
import { Toaster } from "sonner";
import en from "../../messages/en.json";
import pt from "../../messages/pt-BR.json";

const CATALOGS = { en: { ...bridgeEn, ...en }, "pt-BR": { ...bridgePt, ...pt } } as const;

/** The language is the launcher's own setting (Settings → Language), so the catalog follows the snapshot. */
function Localized({ children }: { children: ReactNode }) {
  const { snapshot, api } = useBridge();
  const locale = snapshot?.settings.language ?? "en";
  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);
  // The server's branding may name an accent; the theme's own choices are the only ones honoured.
  useEffect(() => {
    void api?.getBranding().then((b) => {
      document.documentElement.dataset.daintyAccent = parseAccent(b.accent);
    });
  }, [api]);
  return (
    <NextIntlClientProvider
      locale={locale}
      messages={CATALOGS[locale]}
      timeZone={Intl.DateTimeFormat().resolvedOptions().timeZone}
    >
      {children}
      <BootSplash ready={snapshot !== null} />
    </NextIntlClientProvider>
  );
}

export function DesktopProviders({ children }: { children: ReactNode }) {
  return (
    <BridgeProvider>
      <DaintyThemeProvider persistence="local">
        <Localized>
          <LinkProvider component={NextLink}>
            {children}
            <Toaster richColors position="bottom-right" />
          </LinkProvider>
        </Localized>
      </DaintyThemeProvider>
    </BridgeProvider>
  );
}

"use client";

import { LinkProvider } from "@astryxdesign/core/Link";
import bridgeEn from "@dainty/bridge/messages/en.json";
import bridgePt from "@dainty/bridge/messages/pt-BR.json";
import { BridgeProvider, useBridge } from "@dainty/bridge/react";
import { DaintyThemeProvider } from "@dainty/theme";
import NextLink from "next/link";
import { NextIntlClientProvider } from "next-intl";
import { type ReactNode, useEffect } from "react";
import { Toaster } from "sonner";
import en from "../../messages/en.json";
import pt from "../../messages/pt-BR.json";

const CATALOGS = { en: { ...bridgeEn, ...en }, "pt-BR": { ...bridgePt, ...pt } } as const;

function Localized({ children }: { children: ReactNode }) {
  const { snapshot } = useBridge();
  const locale = snapshot?.settings.language ?? "en";
  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);
  return (
    <NextIntlClientProvider
      locale={locale}
      messages={CATALOGS[locale]}
      timeZone={Intl.DateTimeFormat().resolvedOptions().timeZone}
    >
      {children}
    </NextIntlClientProvider>
  );
}

export function DesktopProviders({ children }: { children: ReactNode }) {
  return (
    <BridgeProvider host>
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

"use client";

import { LinkProvider } from "@astryxdesign/core/Link";
import { DaintyThemeProvider, type ThemeMode } from "@dainty/theme";
import NextLink from "next/link";
import { type AbstractIntlMessages, type Locale, NextIntlClientProvider } from "next-intl";
import type { ReactNode } from "react";
import { Toaster } from "sonner";
import { TimeZoneSync } from "@/components/TimeZoneSync";

export default function Providers({
  children,
  initialMode,
  locale,
  messages,
  timeZone,
}: {
  children: ReactNode;
  initialMode: ThemeMode;
  locale: Locale;
  messages: AbstractIntlMessages;
  timeZone: string;
}) {
  return (
    // Explicit: vinext's SSR environment does not see next-intl's request config, and left to infer
    // the SSR pass throws.
    <NextIntlClientProvider locale={locale} messages={messages} timeZone={timeZone}>
      <TimeZoneSync timeZone={timeZone} />
      <DaintyThemeProvider initialMode={initialMode}>
        {/* Every Astryx link stays a client-side navigation. */}
        <LinkProvider component={NextLink}>
          {children}
          <Toaster richColors position="bottom-right" />
        </LinkProvider>
      </DaintyThemeProvider>
    </NextIntlClientProvider>
  );
}

import { parseThemeMode, THEME_COOKIE, themeAttr, themeColor } from "@dainty/theme/mode";
import type { Metadata, Viewport } from "next";
import { cookies } from "next/headers";
import { getLocale, getMessages, getTimeZone, getTranslations } from "next-intl/server";
import type { ReactNode } from "react";
import { getSettings } from "@/lib/settings";
import Providers from "./providers";
import "./globals.css";

export async function generateViewport(): Promise<Viewport> {
  const mode = parseThemeMode((await cookies()).get(THEME_COOKIE)?.value);
  return { themeColor: themeColor(mode) };
}

export async function generateMetadata(): Promise<Metadata> {
  const t = await getTranslations("common");
  const { serverName } = getSettings();
  return {
    title: { default: t("appTitle", { server: serverName }), template: `%s · ${serverName}` },
    description: t("metaDescription"),
    robots: { index: false, follow: false },
  };
}

export default async function RootLayout({ children }: { children: ReactNode }) {
  const mode = parseThemeMode((await cookies()).get(THEME_COOKIE)?.value);
  const [locale, messages, timeZone] = await Promise.all([
    getLocale(),
    getMessages(),
    getTimeZone(),
  ]);
  const { accent } = getSettings();
  return (
    // data-theme from the cookie so the first paint is in the right mode; omitted for "system".
    <html
      lang={locale}
      data-theme={themeAttr(mode)}
      data-dainty-accent={accent}
      suppressHydrationWarning
    >
      <body>
        <Providers initialMode={mode} locale={locale} messages={messages} timeZone={timeZone}>
          {children}
        </Providers>
      </body>
    </html>
  );
}

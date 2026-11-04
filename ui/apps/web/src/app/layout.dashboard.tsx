import {
  APPEARANCE_COOKIE,
  appearanceAttrs,
  parseAppearance,
  parseThemeMode,
  THEME_COOKIE,
  themeAttr,
  themeColor,
} from "@dainty/theme/mode";
import type { Metadata, Viewport } from "next";
import { cookies } from "next/headers";
import { getLocale, getMessages, getTimeZone, getTranslations } from "next-intl/server";
import type { ReactNode } from "react";
import { getSettings } from "@/lib/settings";
import Providers from "./providers";
import "./globals.css";

export async function generateViewport(): Promise<Viewport> {
  const jar = await cookies();
  const mode = parseThemeMode(jar.get(THEME_COOKIE)?.value);
  return {
    themeColor: themeColor(mode, parseAppearance(jar.get(APPEARANCE_COOKIE)?.value).palette),
  };
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
  const jar = await cookies();
  const mode = parseThemeMode(jar.get(THEME_COOKIE)?.value);
  const appearance = parseAppearance(jar.get(APPEARANCE_COOKIE)?.value);
  const [locale, messages, timeZone] = await Promise.all([
    getLocale(),
    getMessages(),
    getTimeZone(),
  ]);
  const { accent } = getSettings();
  return (
    // From the cookies, so the first paint is in the viewer's mode and look; "system" omits data-theme.
    <html
      lang={locale}
      data-theme={themeAttr(mode)}
      {...appearanceAttrs(appearance, accent)}
      suppressHydrationWarning
    >
      <body>
        <Providers
          initialMode={mode}
          initialAppearance={appearance}
          accent={accent}
          locale={locale}
          messages={messages}
          timeZone={timeZone}
        >
          {children}
        </Providers>
      </body>
    </html>
  );
}

import { cookies } from "next/headers";
import { getRequestConfig } from "next-intl/server";
import { LOCALE, resolveTimeZone, TIME_ZONE_COOKIE } from "@/lib/locale";

/**
 * vinext finds this file by path and registers the `next-intl/config` alias itself; there is no
 * createNextIntlPlugin, which reads next/package.json and throws under Vite. Moving it disables next-intl.
 */
export default getRequestConfig(async () => {
  const cookieStore = await cookies();
  return {
    locale: LOCALE,
    messages: (await import("../../messages/en.json")).default,
    timeZone: resolveTimeZone(cookieStore.get(TIME_ZONE_COOKIE)?.value),
  };
});

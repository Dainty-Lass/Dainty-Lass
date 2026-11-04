import type bridge from "@dainty/bridge/messages/en.json";
import type dashboard from "../../messages/en.json";
import type launcher from "../../messages/launcher/en.json";
import type manager from "../../messages/manager/en.json";

// One type for every mode: each mode loads only its own part (layout.dashboard.tsx, DesktopProviders).
declare module "next-intl" {
  interface AppConfig {
    Locale: "en" | "pt-BR";
    Messages: typeof dashboard &
      typeof bridge & { launcher: typeof launcher; manager: typeof manager };
  }
}

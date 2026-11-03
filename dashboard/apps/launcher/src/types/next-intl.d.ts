import type bridge from "@dainty/bridge/messages/en.json";
import type messages from "../../messages/en.json";

declare module "next-intl" {
  interface AppConfig {
    Locale: "en" | "pt-BR";
    Messages: typeof messages & typeof bridge;
  }
}

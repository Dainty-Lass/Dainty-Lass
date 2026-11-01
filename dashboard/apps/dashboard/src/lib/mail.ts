import { createTranslator } from "next-intl";
import { createTransport, type Transporter } from "nodemailer";
import { config } from "@/lib/config";
import { LOCALE } from "@/lib/locale";
import messages from "../../messages/en.json";

let transport: Transporter | null = null;

/**
 * Sends a sign-in code. Without SMTP, a dev box prints it to the console, the one place a code is ever
 * written down; outside dev, email sign-in is not offered at all (config.emailSignInAvailable).
 */
export async function sendSignInCode(to: string, code: string, serverName: string): Promise<void> {
  if (!config.mail.smtpUrl || !config.mail.from) {
    if (config.devLogin) console.info(`[mail] dev sign-in code for ${to}: ${code}`);
    return;
  }
  // Outside a request, so no getTranslations: the catalog is read directly.
  const t = createTranslator({ locale: LOCALE, messages, namespace: "mail.signInCode" });
  transport ??= createTransport(config.mail.smtpUrl);
  await transport.sendMail({
    from: config.mail.from,
    to,
    subject: t("subject", { code, server: serverName }),
    text: t("body", { code, server: serverName }),
  });
}

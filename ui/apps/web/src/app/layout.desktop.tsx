import type { Metadata, Viewport } from "next";
import type { ReactNode } from "react";
import { DesktopProviders } from "@/components/desktop/DesktopProviders";
import { MODE } from "@/mode";
import "./globals.css";

export const metadata: Metadata = {
  title: MODE === "manager" ? "Dainty Lass Server" : "Dainty Lass",
};
export const viewport: Viewport = { colorScheme: "light dark" };

// Static: a Tauri shell serves this file, so nothing here may read a request.
export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body>
        <DesktopProviders>{children}</DesktopProviders>
      </body>
    </html>
  );
}

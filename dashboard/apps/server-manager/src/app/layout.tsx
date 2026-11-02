import type { Metadata, Viewport } from "next";
import type { ReactNode } from "react";
import { DesktopProviders } from "@/components/DesktopProviders";
import "./globals.css";

export const metadata: Metadata = { title: "DaintyLass Server" };
export const viewport: Viewport = { colorScheme: "light dark" };

// Static: the Tauri server GUI serves this file, so nothing here may read a request.
export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body>
        <DesktopProviders>{children}</DesktopProviders>
      </body>
    </html>
  );
}

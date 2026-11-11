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
      {process.env.NODE_ENV === "development" && (
        <head>
          {/* Standalone React DevTools (`bun run devtools`): blocking, so it hooks in before React loads. Dev only,
              since the shells' CSP refuses it; with the app closed the load just fails. */}
          <script src="http://localhost:8097" />
        </head>
      )}
      <body>
        <DesktopProviders>{children}</DesktopProviders>
      </body>
    </html>
  );
}

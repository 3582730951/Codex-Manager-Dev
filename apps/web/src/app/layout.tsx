import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Codex 管理台",
  description: "中文优先、图形化优先的 Codex Manager 控制台。"
};

export default function RootLayout({
  children
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="zh-CN">
      <body>{children}</body>
    </html>
  );
}

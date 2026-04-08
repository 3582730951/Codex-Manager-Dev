import { headers } from "next/headers";
import { getAdminHealth, getDashboardSnapshot } from "@/lib/dashboard";
import { DashboardApp } from "@/components/dashboard/DashboardApp";
import { getOpenAiCallbackPublicUrl } from "@/lib/openai-oauth";
import { getGatewayPublicBaseUrl } from "@/lib/gateway-public-url";

export const dynamic = "force-dynamic";

export default async function Page() {
  const requestHeaders = await headers();
  const acceptLanguage = requestHeaders.get("accept-language") ?? "";
  const initialLanguage = acceptLanguage.toLowerCase().includes("zh")
    ? "zh"
    : "en";
  const [snapshot, health] = await Promise.all([
    getDashboardSnapshot(),
    getAdminHealth()
  ]);

  return (
    <DashboardApp
      callbackUrl={getOpenAiCallbackPublicUrl()}
      gatewayBaseUrl={getGatewayPublicBaseUrl()}
      health={health}
      initialLanguage={initialLanguage}
      snapshot={snapshot}
    />
  );
}

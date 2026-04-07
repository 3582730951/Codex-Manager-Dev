import {
  dashboardFallback,
  type DashboardSnapshot
} from "@codex-manager/contracts";

const adminOrigin =
  process.env.SERVER_ADMIN_ORIGIN ?? "http://127.0.0.1:8081";

export async function getDashboardSnapshot(): Promise<DashboardSnapshot> {
  try {
    const response = await fetch(`${adminOrigin}/api/v1/dashboard`, {
      cache: "no-store",
      headers: {
        "x-cmgr-dashboard-client": "web-ssr"
      }
    });
    if (!response.ok) {
      return dashboardFallback;
    }
    return (await response.json()) as DashboardSnapshot;
  } catch {
    return dashboardFallback;
  }
}


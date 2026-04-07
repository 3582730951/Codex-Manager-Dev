import {
  dashboardFallback,
  type BrowserTask,
  type DashboardSnapshot
} from "@codex-manager/contracts";

const adminOrigin =
  process.env.SERVER_ADMIN_ORIGIN ?? "http://127.0.0.1:8081";

export interface TenantView {
  id: string;
  slug: string;
  name: string;
  createdAt: string;
}

export interface AdminHealthSnapshot {
  service: string;
  status: string;
  storageMode: string;
  postgresConnected: boolean;
  redisConnected: boolean;
  redisChannel: string;
  instanceId: string;
  postgresUrl: string;
  redisUrl: string;
  browserAssistUrl: string;
  directProxyConfigured: boolean;
  warpProxyConfigured: boolean;
  browserAssistDirectProxyConfigured: boolean;
  browserAssistWarpProxyConfigured: boolean;
}

export interface OpenAiLoginStartResult {
  loginId: string;
  authUrl: string;
  redirectUri: string;
}

export interface OpenAiLoginSession {
  loginId: string;
  tenantId: string;
  label: string | null;
  note: string | null;
  redirectUri: string;
  authUrl: string;
  status: string;
  error: string | null;
  importedAccountId: string | null;
  importedAccountLabel: string | null;
  createdAt: string;
  updatedAt: string;
}

const healthFallback: AdminHealthSnapshot = {
  service: "server-admin",
  status: "offline",
  storageMode: "unknown",
  postgresConnected: false,
  redisConnected: false,
  redisChannel: "cmgr:control-events",
  instanceId: "offline",
  postgresUrl: "n/a",
  redisUrl: "n/a",
  browserAssistUrl: "n/a",
  directProxyConfigured: false,
  warpProxyConfigured: false,
  browserAssistDirectProxyConfigured: false,
  browserAssistWarpProxyConfigured: false
};

async function fetchAdmin<T>(
  path: string,
  init?: RequestInit
): Promise<T> {
  const response = await fetch(`${adminOrigin}${path}`, {
    cache: "no-store",
    ...init,
    headers: {
      "content-type": "application/json",
      "x-cmgr-dashboard-client": "web-ssr",
      ...(init?.headers ?? {})
    }
  });

  if (!response.ok) {
    const raw = await response.text().catch(() => "");
    let message = `管理接口返回 ${response.status}`;
    if (raw) {
      try {
        const payload = JSON.parse(raw) as {
          error?: { message?: string };
          message?: string;
        };
        message =
          payload.error?.message ??
          payload.message ??
          message;
      } catch {
        message = raw;
      }
    }
    throw new Error(message);
  }

  if (response.status === 204) {
    return null as T;
  }

  return (await response.json()) as T;
}

export async function getDashboardSnapshot(): Promise<DashboardSnapshot> {
  try {
    return await fetchAdmin<DashboardSnapshot>("/api/v1/dashboard");
  } catch {
    return dashboardFallback;
  }
}

export async function getTenants(): Promise<TenantView[]> {
  try {
    return await fetchAdmin<TenantView[]>("/api/v1/tenants");
  } catch {
    return [];
  }
}

export async function getAdminHealth(): Promise<AdminHealthSnapshot> {
  try {
    return await fetchAdmin<AdminHealthSnapshot>("/health");
  } catch {
    return healthFallback;
  }
}

export async function createTenant(payload: {
  slug: string;
  name: string;
}) {
  return fetchAdmin<TenantView>("/api/v1/tenants", {
    method: "POST",
    body: JSON.stringify(payload)
  });
}

export async function importAccount(payload: {
  tenantId: string;
  label: string;
  models: string[];
  baseUrl?: string;
  bearerToken?: string;
  chatgptAccountId?: string;
  extraHeaders?: Array<[string, string]>;
  quotaHeadroom?: number;
  quotaHeadroom5h?: number;
  quotaHeadroom7d?: number;
  healthScore?: number;
  egressStability?: number;
}) {
  return fetchAdmin<Record<string, unknown>>("/api/v1/accounts/import", {
    method: "POST",
    body: JSON.stringify(payload)
  });
}

export async function submitBrowserTask(
  kind: "login" | "recover",
  payload: {
    accountId?: string;
    notes?: string;
    loginUrl?: string;
    headless?: boolean;
    provider?: string;
    email?: string;
    password?: string;
    otpCode?: string;
    routeMode?: "direct" | "warp";
  }
) {
  return fetchAdmin<BrowserTask>(`/api/v1/browser/tasks/${kind}`, {
    method: "POST",
    body: JSON.stringify(payload)
  });
}

export async function startOpenAiLogin(payload: {
  tenantId?: string;
  label?: string;
  note?: string;
  redirectUri: string;
  models?: string[];
  baseUrl?: string;
}) {
  return fetchAdmin<OpenAiLoginStartResult>("/api/v1/openai/login/start", {
    method: "POST",
    body: JSON.stringify(payload)
  });
}

export async function getOpenAiLoginStatus(loginId: string) {
  return fetchAdmin<OpenAiLoginSession>(`/api/v1/openai/login/${loginId}`);
}

export async function completeOpenAiLogin(payload: {
  state: string;
  code: string;
  redirectUri?: string;
}) {
  return fetchAdmin<Record<string, unknown>>("/api/v1/openai/login/complete", {
    method: "POST",
    body: JSON.stringify(payload)
  });
}

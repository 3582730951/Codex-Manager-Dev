export type AccountRouteMode = "direct" | "warp";
export type GatewayUserRole = "admin" | "viewer";

export interface CacheMetrics {
  cachedTokens: number;
  replayTokens: number;
  prefixHitRatio: number;
  warmupRoi: number;
  staticPrefixTokens: number;
}

export interface AccountSummary {
  id: string;
  tenantId: string;
  label: string;
  models: string[];
  currentMode: AccountRouteMode;
  routeMode: AccountRouteMode;
  cooldownLevel: number;
  cooldownUntil: string | null;
  directCfStreak: number;
  warpCfStreak: number;
  successStreak: number;
  quotaHeadroom: number;
  quotaHeadroom5h: number;
  quotaHeadroom7d: number;
  nearQuotaGuardEnabled: boolean;
  healthScore: number;
  egressStability: number;
  inflight: number;
  capacity: number;
  hasCredential: boolean;
  baseUrl: string | null;
  chatgptAccountId: string | null;
  egressGroup: string;
  proxyEnabled: boolean;
}

export interface LeaseView {
  principalId: string;
  accountId: string;
  accountLabel: string;
  model: string;
  routeMode: AccountRouteMode;
  generation: number;
  activeSubagents: number;
  lastUsedAt: string;
}

export interface CfIncident {
  id: string;
  accountId: string;
  accountLabel: string;
  routeMode: AccountRouteMode;
  severity: string;
  happenedAt: string;
  cooldownLevel: number;
}

export interface ServiceTopologyNode {
  name: string;
  purpose: string;
  hotPath: boolean;
  port: number;
}

export interface BrowserTask {
  id: string;
  kind: string;
  accountId: string | null;
  accountLabel: string | null;
  provider: string | null;
  routeMode: AccountRouteMode | null;
  status: string;
  createdAt: string;
  updatedAt: string;
  notes: string | null;
  profileDir: string | null;
  screenshotPath: string | null;
  storageStatePath: string | null;
  finalUrl: string | null;
  lastError: string | null;
  stepCount: number;
}

export interface GatewayUserView {
  id: string;
  tenantId: string;
  name: string;
  email: string;
  role: GatewayUserRole;
  tokenPreview: string;
  token: string;
  defaultModel: string | null;
  reasoningEffort: string | null;
  forceModelOverride: boolean;
  forceReasoningEffort: boolean;
  requestCount: number;
  estimatedSpendUsd: number;
  lastUsedAt: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface RequestLogUsage {
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  totalTokens: number;
}

export interface RequestLogEntry {
  id: string;
  apiKeyId: string;
  tenantId: string;
  userName: string;
  userEmail: string;
  principalId: string;
  accountId: string;
  accountLabel: string;
  method: string;
  endpoint: string;
  requestedModel: string;
  effectiveModel: string;
  reasoningEffort: string | null;
  routeMode: AccountRouteMode;
  statusCode: number;
  usage: RequestLogUsage;
  estimatedCostUsd: number | null;
  createdAt: string;
}

export interface BillingSummary {
  totalSpendUsd: number;
  totalRequests: number;
  totalInputTokens: number;
  totalCachedInputTokens: number;
  totalOutputTokens: number;
  totalTokens: number;
  pricedRequests: number;
}

export interface DashboardSnapshot {
  title: string;
  subtitle: string;
  topology: ServiceTopologyNode[];
  cacheMetrics: CacheMetrics;
  accounts: AccountSummary[];
  leases: LeaseView[];
  cfIncidents: CfIncident[];
  browserTasks: BrowserTask[];
  users: GatewayUserView[];
  requestLogs: RequestLogEntry[];
  billing: BillingSummary;
  modelCatalog: string[];
  counts: {
    tenants: number;
    accounts: number;
    users: number;
    activeLeases: number;
    warpAccounts: number;
    browserTasks: number;
  };
}

export const dashboardFallback: DashboardSnapshot = {
  title: "Codex 管理台",
  subtitle: "路由、缓存、恢复，一屏看完。",
  topology: [
    {
      name: "web",
      purpose: "中文前台界面",
      hotPath: false,
      port: 3000
    },
    {
      name: "server:data",
      purpose: "OpenAI 兼容网关",
      hotPath: true,
      port: 8080
    },
    {
      name: "server:admin",
      purpose: "控制面与观测面",
      hotPath: false,
      port: 8081
    },
    {
      name: "browser-assist",
      purpose: "登录与挑战恢复",
      hotPath: false,
      port: 8090
    }
  ],
  cacheMetrics: {
    cachedTokens: 0,
    replayTokens: 0,
    prefixHitRatio: 0,
    warmupRoi: 0,
    staticPrefixTokens: 0
  },
  accounts: [
    {
      id: "acc_demo_1",
      tenantId: "demo",
      label: "子午线",
      models: ["gpt-5.4", "gpt-5.3-codex"],
      currentMode: "direct",
      routeMode: "direct",
      cooldownLevel: 0,
      cooldownUntil: null,
      directCfStreak: 0,
      warpCfStreak: 0,
      successStreak: 12,
      quotaHeadroom: 0.92,
      quotaHeadroom5h: 0.92,
      quotaHeadroom7d: 0.88,
      nearQuotaGuardEnabled: false,
      healthScore: 0.96,
      egressStability: 0.88,
      inflight: 0,
      capacity: 4,
      hasCredential: false,
      baseUrl: null,
      chatgptAccountId: null,
      egressGroup: "direct-native",
      proxyEnabled: false
    }
  ],
  leases: [
    {
      principalId: "tenant:demo/principal:atlas-shell",
      accountId: "acc_demo_1",
      accountLabel: "子午线",
      model: "gpt-5.4",
      routeMode: "direct",
      generation: 8,
      activeSubagents: 3,
      lastUsedAt: new Date().toISOString()
    },
    {
      principalId: "tenant:demo/principal:review-bot",
      accountId: "acc_demo_2",
      accountLabel: "西风翼",
      model: "gpt-5.4",
      routeMode: "warp",
      generation: 3,
      activeSubagents: 1,
      lastUsedAt: new Date().toISOString()
    }
  ],
  cfIncidents: [
    {
      id: "incident_demo_1",
      accountId: "acc_demo_2",
      accountLabel: "西风翼",
      routeMode: "warp",
      severity: "cooldown",
      happenedAt: new Date().toISOString(),
      cooldownLevel: 3
    }
  ],
  browserTasks: [
    {
      id: "task_demo_1",
      kind: "recover",
      accountId: "acc_demo_2",
      accountLabel: "西风翼",
      provider: "openai",
      routeMode: "warp",
      status: "completed",
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      notes: "warp 恢复演练",
      profileDir: "/tmp/cmgr-browser-assist/acc_demo_2",
      screenshotPath: null,
      storageStatePath: "/tmp/cmgr-browser-assist/acc_demo_2/recover.storage-state.json",
      finalUrl: "https://chatgpt.com/",
      lastError: null,
      stepCount: 3
    }
  ],
  users: [
    {
      id: "usr-demo-key",
      tenantId: "demo",
      name: "Demo User",
      email: "demo@codex.local",
      role: "admin",
      tokenPreview: "cmgr_d…key",
      token: "cmgr_demo_full_key",
      defaultModel: "gpt-5.4",
      reasoningEffort: "high",
      forceModelOverride: false,
      forceReasoningEffort: false,
      requestCount: 6,
      estimatedSpendUsd: 0.42,
      lastUsedAt: new Date().toISOString(),
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString()
    }
  ],
  requestLogs: [
    {
      id: "log_demo_1",
      apiKeyId: "usr-demo-key",
      tenantId: "demo",
      userName: "Demo User",
      userEmail: "demo@codex.local",
      principalId: "tenant:demo/principal:atlas-shell",
      accountId: "acc_demo_1",
      accountLabel: "子午线",
      method: "POST",
      endpoint: "/v1/responses",
      requestedModel: "gpt-5.4",
      effectiveModel: "gpt-5.4",
      reasoningEffort: "high",
      routeMode: "direct",
      statusCode: 200,
      usage: {
        inputTokens: 4200,
        cachedInputTokens: 1200,
        outputTokens: 980,
        totalTokens: 5180
      },
      estimatedCostUsd: 0.0318,
      createdAt: new Date().toISOString()
    }
  ],
  billing: {
    totalSpendUsd: 0.42,
    totalRequests: 6,
    totalInputTokens: 26000,
    totalCachedInputTokens: 7000,
    totalOutputTokens: 5400,
    totalTokens: 31400,
    pricedRequests: 6
  },
  modelCatalog: ["gpt-5.4", "gpt-5.3-codex", "gpt-5.2"],
  counts: {
    tenants: 1,
    accounts: 4,
    users: 1,
    activeLeases: 2,
    warpAccounts: 1,
    browserTasks: 1
  }
};

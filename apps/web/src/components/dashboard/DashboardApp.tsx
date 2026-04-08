"use client";

import { useEffect, useMemo, useState } from "react";
import type {
  DashboardSnapshot,
  GatewayUserRole,
  GatewayUserView
} from "@codex-manager/contracts";
import {
  Activity,
  ArrowUpRight,
  Bell,
  BellOff,
  Bot,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Clock3,
  Copy,
  Database,
  DollarSign,
  FileText,
  GaugeCircle,
  KeyRound,
  LayoutDashboard,
  MoonStar,
  MoreHorizontal,
  Plus,
  RefreshCw,
  Search,
  Settings,
  Shield,
  ShieldAlert,
  Sparkles,
  SunMedium,
  Terminal,
  TriangleAlert,
  User,
  Users
} from "lucide-react";
import { Area, AreaChart, ResponsiveContainer, Tooltip } from "recharts";
import { AccountAddDrawer } from "@/components/dashboard/AccountAddDrawer";
import {
  UserCreateModal,
  type CreatedGatewayUser
} from "@/components/dashboard/UserCreateModal";

type HealthSummary = {
  status: string;
  storageMode: string;
  postgresConnected: boolean;
  redisConnected: boolean;
  browserAssistUrl: string;
};

type DashboardAppProps = {
  snapshot: DashboardSnapshot;
  health: HealthSummary;
  initialLanguage?: Language;
  callbackUrl: string;
  gatewayBaseUrl: string;
};

type ActiveView =
  | "overview"
  | "accounts"
  | "users"
  | "alerts"
  | "logs"
  | "config";

type AccountFilter = "all" | "active" | "low" | "protected";
type Language = "zh" | "en";
type ThemeMode = "dark" | "light";
type BannerTone = "ok" | "error" | "neutral";

type ToggleRowProps = {
  checked: boolean;
  description: string;
  label: string;
  onChange: () => void;
  theme: ThemeMode;
};

type SidebarItemProps = {
  active: boolean;
  expanded: boolean;
  icon: typeof LayoutDashboard;
  label: string;
  onClick: () => void;
  theme: ThemeMode;
};

type SectionCardProps = {
  actions?: React.ReactNode;
  children: React.ReactNode;
  icon: typeof LayoutDashboard;
  subtitle?: string;
  theme: ThemeMode;
  title: string;
};

type EmptyStateProps = {
  icon: typeof BellOff;
  theme: ThemeMode;
  title: string;
};

type BannerState = {
  tone: BannerTone;
  message: string;
} | null;

type RecentCliInstance = {
  principalId: string;
  affinityId: string;
  requestCount: number;
  totalTokens: number;
  estimatedSpendUsd: number;
  lastUsedAt: string;
  lastEndpoint: string;
  lastModel: string;
  lastAccountLabel: string;
  routeMode: DashboardSnapshot["accounts"][number]["routeMode"];
  statusCode: number;
  activeLease: boolean;
  activeSubagents: number;
};

type UserUsageEntry = {
  user: GatewayUserView;
  totalTokens: number;
  recentInstances: RecentCliInstance[];
};

type UserPolicyCardProps = {
  availableModels: string[];
  compact?: boolean;
  gatewayBaseUrl: string;
  language: Language;
  latestIssuedToken?: string | null;
  onSave: (
    userId: string,
    payload: {
      name?: string;
      email?: string;
      role?: GatewayUserRole;
      defaultModel?: string | null;
      reasoningEffort?: string | null;
      forceModelOverride?: boolean;
      forceReasoningEffort?: boolean;
    }
  ) => Promise<void>;
  recentInstances: RecentCliInstance[];
  saving: boolean;
  tokenUsage?: number;
  theme: ThemeMode;
  user: GatewayUserView;
};

const iconStroke = 1.5;
const themeStorageKey = "cmgr-dashboard-theme";
const languageStorageKey = "cmgr-dashboard-language";

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

function clamp01(value: number) {
  return Math.min(1, Math.max(0, value));
}

function percent(value: number) {
  return Math.round(clamp01(value) * 100);
}

function shortId(value: string) {
  if (value.length <= 18) {
    return value;
  }
  return `${value.slice(0, 7)}...${value.slice(-4)}`;
}

function initialsForName(value: string) {
  const tokens = value
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2);
  if (tokens.length === 0) {
    return "AI";
  }
  return tokens.map((token) => token[0]?.toUpperCase() ?? "").join("");
}

function formatUsd(value: number, language: Language) {
  return new Intl.NumberFormat(language === "zh" ? "zh-CN" : "en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: value < 10 ? 3 : 2
  }).format(value);
}

function formatNumber(value: number, language: Language) {
  return new Intl.NumberFormat(language === "zh" ? "zh-CN" : "en-US").format(
    value
  );
}

function relativeTime(value: string | null, language: Language) {
  if (!value) {
    return language === "zh" ? "暂无" : "Never";
  }

  const target = new Date(value).getTime();
  if (Number.isNaN(target)) {
    return "--";
  }

  const delta = Date.now() - target;
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;

  if (delta < hour) {
    const amount = Math.max(1, Math.round(delta / minute));
    return language === "zh" ? `${amount} 分钟前` : `${amount}m ago`;
  }
  if (delta < day) {
    const amount = Math.max(1, Math.round(delta / hour));
    return language === "zh" ? `${amount} 小时前` : `${amount}h ago`;
  }
  const amount = Math.max(1, Math.round(delta / day));
  return language === "zh" ? `${amount} 天前` : `${amount}d ago`;
}

function formatDateTime(value: string | null, language: Language) {
  if (!value) {
    return "--";
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "--";
  }

  return new Intl.DateTimeFormat(language === "zh" ? "zh-CN" : "en-US", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false
  }).format(date);
}

function affinityIdFromPrincipal(principalId: string) {
  const marker = "/principal:";
  const markerIndex = principalId.indexOf(marker);
  if (markerIndex === -1) {
    return principalId;
  }
  return principalId.slice(markerIndex + marker.length) || principalId;
}

function donutStyle(value: number) {
  const circumference = 2 * Math.PI * 38;
  return {
    strokeDasharray: circumference,
    strokeDashoffset: circumference * (1 - clamp01(value))
  };
}

function accountKind(account: DashboardSnapshot["accounts"][number]) {
  const quotaFloor = Math.min(
    account.quotaHeadroom,
    account.quotaHeadroom5h,
    account.quotaHeadroom7d
  );

  if (account.nearQuotaGuardEnabled || account.cooldownLevel > 0) {
    return "protected";
  }
  if (quotaFloor <= 0.25) {
    return "low";
  }
  return "active";
}

function quotaBarColor(quota: number) {
  if (quota > 0.6) {
    return "bg-emerald-500";
  }
  if (quota > 0.3) {
    return "bg-amber-500";
  }
  return "bg-rose-500";
}

function deriveModelCatalog(snapshot: DashboardSnapshot) {
  const models = new Set(snapshot.modelCatalog);
  for (const account of snapshot.accounts) {
    for (const model of account.models) {
      if (model.trim()) {
        models.add(model.trim());
      }
    }
  }
  return Array.from(models).sort();
}

function buildRequestSeries(
  logs: DashboardSnapshot["requestLogs"],
  fallbackBase: number
) {
  if (logs.length === 0) {
    return [
      { label: "09:00", value: Math.round(fallbackBase * 0.54) },
      { label: "10:00", value: Math.round(fallbackBase * 0.72) },
      { label: "11:00", value: Math.round(fallbackBase * 0.64) },
      { label: "12:00", value: Math.round(fallbackBase * 0.88) },
      { label: "13:00", value: Math.round(fallbackBase * 0.96) },
      { label: "14:00", value: Math.round(fallbackBase * 1.08) },
      { label: "15:00", value: Math.round(fallbackBase * 0.9) }
    ];
  }

  const buckets = new Map<string, number>();
  const ordered = [...logs].sort(
    (left, right) =>
      new Date(left.createdAt).getTime() - new Date(right.createdAt).getTime()
  );

  for (const log of ordered) {
    const date = new Date(log.createdAt);
    const label = `${String(date.getHours()).padStart(2, "0")}:00`;
    buckets.set(label, (buckets.get(label) ?? 0) + 1);
  }

  return Array.from(buckets.entries())
    .slice(-8)
    .map(([label, value]) => ({
      label,
      value
    }));
}

async function copyText(value: string) {
  await navigator.clipboard.writeText(value);
}

function SidebarItem({
  active,
  expanded,
  icon: Icon,
  label,
  onClick,
  theme
}: SidebarItemProps) {
  const isDark = theme === "dark";

  return (
    <button
      className={cx(
        "group flex h-12 items-center rounded-[20px] text-sm font-medium transition-all duration-200",
        expanded ? "w-full gap-3 px-3" : "mx-auto w-12 justify-center px-0",
        active
          ? isDark
            ? "bg-sky-500/12 text-sky-200 shadow-soft"
            : "bg-sky-50 text-sky-600 shadow-soft"
          : isDark
            ? "text-zinc-400 hover:bg-white/[0.04] hover:text-zinc-100"
            : "text-zinc-500 hover:bg-white/80 hover:text-zinc-900"
      )}
      onClick={onClick}
      title={expanded ? undefined : label}
      type="button"
    >
      <span
        className={cx(
          "flex h-9 w-9 shrink-0 items-center justify-center rounded-2xl transition-all duration-200",
          active
            ? isDark
              ? "bg-sky-400/16 text-sky-200"
              : "bg-sky-100 text-sky-600"
            : isDark
              ? "bg-white/[0.05] text-zinc-400 group-hover:bg-white/[0.08]"
              : "bg-zinc-100 text-zinc-500 group-hover:bg-zinc-200/80"
        )}
      >
        <Icon size={18} strokeWidth={iconStroke} />
      </span>
      <span
        className={cx(
          "overflow-hidden whitespace-nowrap text-left transition-all duration-200",
          expanded ? "max-w-[160px] opacity-100" : "max-w-0 opacity-0"
        )}
      >
        {label}
      </span>
    </button>
  );
}

function SectionCard({
  actions,
  children,
  icon: Icon,
  subtitle,
  theme,
  title
}: SectionCardProps) {
  const isDark = theme === "dark";

  return (
    <section
      className={cx(
        "rounded-[32px] border p-6 shadow-panel backdrop-blur-xl",
        isDark
          ? "border-white/10 bg-[#11141b]/82"
          : "border-white/70 bg-white/85"
      )}
    >
      <header className="mb-5 flex items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <span
            className={cx(
              "flex h-11 w-11 items-center justify-center rounded-2xl",
              isDark ? "bg-white/[0.05] text-zinc-300" : "bg-zinc-100 text-zinc-600"
            )}
          >
            <Icon size={18} strokeWidth={iconStroke} />
          </span>
          <div>
            <h2
              className={cx(
                "text-xl font-semibold tracking-tight",
                isDark ? "text-zinc-50" : "text-zinc-900"
              )}
            >
              {title}
            </h2>
            {subtitle ? (
              <p
                className={cx(
                  "mt-1 text-xs",
                  isDark ? "text-zinc-500" : "text-zinc-500"
                )}
              >
                {subtitle}
              </p>
            ) : null}
          </div>
        </div>
        {actions}
      </header>
      {children}
    </section>
  );
}

function EmptyState({ icon: Icon, theme, title }: EmptyStateProps) {
  const isDark = theme === "dark";

  return (
    <div
      className={cx(
        "flex min-h-[240px] flex-col items-center justify-center gap-3 rounded-[24px] border border-dashed text-center",
        isDark
          ? "border-white/10 bg-white/[0.02]"
          : "border-zinc-200 bg-zinc-50/80"
      )}
    >
      <span
        className={cx(
          "flex h-16 w-16 items-center justify-center rounded-full shadow-soft",
          isDark ? "bg-white/[0.05] text-zinc-600" : "bg-white text-zinc-300"
        )}
      >
        <Icon size={28} strokeWidth={iconStroke} />
      </span>
      <p className={cx("text-sm", isDark ? "text-zinc-500" : "text-zinc-400")}>
        {title}
      </p>
    </div>
  );
}

function ToggleRow({
  checked,
  description,
  label,
  onChange,
  theme
}: ToggleRowProps) {
  const isDark = theme === "dark";

  return (
    <div
      className={cx(
        "flex items-center justify-between gap-4 rounded-[28px] border p-4 shadow-soft",
        isDark
          ? "border-white/10 bg-white/[0.03]"
          : "border-zinc-200 bg-white"
      )}
    >
      <div>
        <p className={cx("text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
          {label}
        </p>
        <p
          className={cx(
            "mt-1 text-xs leading-5",
            isDark ? "text-zinc-500" : "text-zinc-500"
          )}
        >
          {description}
        </p>
      </div>
      <button
        aria-pressed={checked}
        className={cx(
          "relative h-8 w-14 rounded-full transition-all duration-200",
          checked ? "bg-sky-500" : isDark ? "bg-white/10" : "bg-zinc-300"
        )}
        onClick={onChange}
        type="button"
      >
        <span
          className={cx(
            "absolute top-1 h-6 w-6 rounded-full bg-white shadow-sm transition-all duration-200",
            checked ? "left-7" : "left-1"
          )}
        />
      </button>
    </div>
  );
}

function translationFor(language: Language) {
  return language === "zh"
    ? {
        brandTitle: "AI 账号池",
        brandSubtitle: "Pool Control",
        navOverview: "概览",
        navAccounts: "账号",
        navUsers: "用户",
        navAlerts: "告警",
        navLogs: "日志",
        navConfig: "配置",
        headerKicker: "AI Account Pool",
        headerTitle: "暗色账号池控制台",
        headerDescription:
          "统一账号接入、用户策略、消费追踪和网关连接配置，面向日常运维直接可用。",
        summaryCache: "缓存",
        summaryUsers: "用户",
        summarySpend: "消费",
        summaryHealth: "状态",
        nominal: "正常",
        attention: "关注",
        overview: "概览",
        overviewSub: "缓存、账号状态和消费概况",
        cacheHit: "缓存命中",
        prefixCache: "前缀缓存",
        hit: "命中",
        tokens: "tokens",
        lowQuota: "低额度",
        lowQuotaDesc: "额度接近下限，建议优先排查或切换出口。",
        active: "活跃",
        activeDesc: "当前健康且可路由的账号总数。",
        protected: "保护中",
        protectedDesc: "近额保护或冷却保护已启用。",
        totalSpend: "总消费",
        spendDesc: "按官方模型价格估算，默认美元。",
        requestsOverTime: "请求趋势",
        requests: "请求",
        topUsers: "消费靠前用户",
        pricedRequests: "已计价请求",
        accounts: "账号",
        accountsSub: "支持模型、额度与路由状态",
        all: "全部",
        searchDisabled: "搜索稍后接入",
        quota: "额度",
        route: "出口",
        modelCount: "模型数",
        refreshModels: "刷新模型",
        refreshing: "刷新中...",
        modelsReady: "模型目录已更新",
        addAccount: "添加账号",
        userManagement: "用户管理",
        usersSub: "网关用户、token 消耗与策略控制",
        admin: "管理员",
        viewer: "只读",
        addUser: "添加用户",
        userConfig: "用户配置",
        userConfigHint: "CLI 接入、网关 Key 预览、多 CLI 亲和和模型覆盖都收在这里。",
        policySectionKicker: "可编辑策略",
        policySectionTitle: "身份与策略表单",
        policySectionDesc: "维护用户身份、默认模型与强制覆盖规则。",
        accessSectionKicker: "只读接入",
        accessSectionTitle: "CLI 接入与配置",
        accessSectionDesc: "保留网关地址、Key 预览与可复制的 CLI 配置片段。",
        instancesSectionKicker: "最近活跃",
        instancesSectionTitle: "最近 CLI 实例",
        instancesSectionDesc:
          "按 affinity 聚合最近请求，便于只读查看该用户正在服务的 CLI 实例。",
        noRecentInstances: "还没有观测到这个用户的 CLI 实例请求。",
        activeLease: "活跃租约",
        servedBy: "接入账号",
        lastPath: "最近路径",
        cliInstance: "CLI 实例",
        statusCode: "状态码",
        subagents: "子代理",
        activeUsers: "活跃用户",
        openConfig: "打开配置",
        cliConnect: "下游 CLI 接入",
        cliConnectDesc:
          "新增用户后，将网关地址和该用户的网关 Key 发给 CLI 即可。",
        multiCliTitle: "多 CLI 隔离",
        multiCliDesc:
          "同一把网关 Key 可以给多个下游 CLI 共用，但每个 CLI 都应带上独立 affinity，避免租约和上下文串线。",
        affinityHeader: "亲和 Header",
        affinityExample: "建议值",
        affinityHint:
          "优先使用 x-codex-cli-affinity-id；若未显式提供，网关才会退化到 session_id / subagent。",
        gatewayBase: "网关地址",
        gatewayKey: "网关 Key",
        affinityId: "亲和 ID",
        shellExample: "Shell 示例",
        envPreset: "环境变量",
        codexConfig: "Codex 配置",
        curlExample: "curl 示例",
        copy: "复制",
        copied: "已复制",
        fullKeyOnce: "完整 Key 只会在创建成功时显示一次；现有用户仅保留预览。",
        tokenPreview: "Key 预览",
        requestCount: "请求数",
        estimatedSpend: "消费",
        lastUsed: "最后使用",
        defaultModel: "默认模型",
        useRequestModel: "沿用请求模型",
        reasoning: "推理强度",
        useRequestReasoning: "沿用请求参数",
        forceModel: "强制覆盖模型",
        forceReasoning: "强制覆盖推理",
        save: "保存策略",
        saving: "保存中...",
        saved: "策略已保存",
        noUsers: "暂无网关用户",
        alerts: "告警",
        alertsSub: "账号低额度与保护状态时间线",
        noAlerts: "暂无告警",
        lowQuotaDetected: (id: string) => `账号 ${id} 检测到低额度`,
        alertDescription: (label: string, severity: string, level: number) =>
          `${label} 进入 ${severity.toLowerCase()} 状态，当前冷却等级为 ${level}。`,
        requestLogs: "请求日志",
        requestLogsSub: "真实请求、模型、用户和估算消费",
        noLogs: "暂无请求日志",
        method: "方法",
        endpoint: "路径",
        user: "用户",
        model: "模型",
        cost: "费用",
        systemConfig: "系统配置",
        configSub: "主题、保护策略和基础服务健康",
        protectionMode: "保护模式",
        protectionModeDesc: "当账号接近额度下限时，自动进入受保护路由。",
        autoRefill: "自动补充",
        autoRefillDesc: "在低额度场景下尝试自动补充或切换资源。",
        storage: "存储",
        services: "服务",
        online: "在线",
        offline: "离线",
        theme: "主题",
        darkTheme: "暗夜",
        lightTheme: "浅色",
        drawerKicker: "池内动作",
        drawerTitle: "快速操作",
        syncSnapshot: "账号工作区",
        reviewProtected: "用户策略",
        priorityAccounts: "优先关注账号",
        noPriorityAccounts: "暂无重点账号",
        language: "语言",
        sidebarState: "侧栏",
        collapse: "收起",
        expand: "展开",
        systemReady: "系统正常",
        checkSystem: "需要检查",
        accountsInPool: (count: number) => `池内 ${count} 个账号`,
        userCreated: (name: string) => `用户 ${name} 已创建`,
        userSaved: "用户策略已更新",
        refreshFailed: "刷新模型失败",
        saveFailed: "保存用户失败",
        requestModelHelp: "是否覆盖下游传入模型",
        requestReasoningHelp: "是否覆盖下游传入推理强度"
      }
    : {
        brandTitle: "AI Pool",
        brandSubtitle: "Pool Control",
        navOverview: "Overview",
        navAccounts: "Accounts",
        navUsers: "Users",
        navAlerts: "Alerts",
        navLogs: "Logs",
        navConfig: "Config",
        headerKicker: "AI Account Pool",
        headerTitle: "Dark Pool Console",
        headerDescription:
          "Run intake, user policy, spend tracking, and gateway connection from a single control surface.",
        summaryCache: "Cache",
        summaryUsers: "Users",
        summarySpend: "Spend",
        summaryHealth: "Health",
        nominal: "Nominal",
        attention: "Attention",
        overview: "Overview",
        overviewSub: "Cache, account state, and spend signals",
        cacheHit: "Cache Hit",
        prefixCache: "Prefix Cache",
        hit: "hit",
        tokens: "tokens",
        lowQuota: "Low Quota",
        lowQuotaDesc: "Quota is approaching the lower bound. Review or reroute soon.",
        active: "Active",
        activeDesc: "Healthy accounts currently available for routing.",
        protected: "Protected",
        protectedDesc: "Quota guard or cooldown protection is enabled.",
        totalSpend: "Total Spend",
        spendDesc: "Estimated from official model pricing, in USD.",
        requestsOverTime: "Requests over time",
        requests: "Requests",
        topUsers: "Top spend users",
        pricedRequests: "Priced requests",
        accounts: "Accounts",
        accountsSub: "Supported models, quota, and route state",
        all: "All",
        searchDisabled: "Search coming next",
        quota: "Quota",
        route: "Route",
        modelCount: "Models",
        refreshModels: "Refresh Models",
        refreshing: "Refreshing...",
        modelsReady: "Model catalog updated",
        addAccount: "Add Account",
        userManagement: "User Management",
        usersSub: "Gateway identities, token usage, and policy control",
        admin: "Admin",
        viewer: "Viewer",
        addUser: "Add User",
        userConfig: "User Config",
        userConfigHint:
          "CLI connection, key preview, multi-CLI affinity, and model override all live here.",
        policySectionKicker: "Editable Policy",
        policySectionTitle: "Identity and policy form",
        policySectionDesc: "Manage identity, default model, and forced override rules.",
        accessSectionKicker: "Read-only Access",
        accessSectionTitle: "CLI connection and config",
        accessSectionDesc:
          "Keep the gateway base, key preview, and copy-ready CLI snippets in one place.",
        instancesSectionKicker: "Recent Activity",
        instancesSectionTitle: "Recent CLI instances",
        instancesSectionDesc:
          "Grouped by affinity so you can review the most recent downstream CLI instances in read-only form.",
        noRecentInstances: "No CLI instance activity has been observed for this user yet.",
        activeLease: "Active Lease",
        servedBy: "Served By",
        lastPath: "Last Path",
        cliInstance: "CLI Instance",
        statusCode: "Status",
        subagents: "Subagents",
        activeUsers: "Active Users",
        openConfig: "Open Config",
        cliConnect: "CLI Connection",
        cliConnectDesc:
          "After creating a user, hand the gateway base URL and that user's gateway key to downstream CLI.",
        multiCliTitle: "Multi-CLI Isolation",
        multiCliDesc:
          "One gateway key can be shared across multiple downstream CLIs, but each CLI should send its own affinity id so leases and context stay isolated.",
        affinityHeader: "Affinity Header",
        affinityExample: "Recommended value",
        affinityHint:
          "Prefer x-codex-cli-affinity-id. The gateway only falls back to session_id / subagent when no explicit affinity is provided.",
        gatewayBase: "Gateway Base",
        gatewayKey: "Gateway Key",
        affinityId: "Affinity ID",
        shellExample: "Shell Example",
        envPreset: "Env Vars",
        codexConfig: "Codex Config",
        curlExample: "curl Example",
        copy: "Copy",
        copied: "Copied",
        fullKeyOnce:
          "The full key is shown only once at creation time; existing users keep a preview only.",
        tokenPreview: "Key Preview",
        requestCount: "Requests",
        estimatedSpend: "Spend",
        lastUsed: "Last Used",
        defaultModel: "Default Model",
        useRequestModel: "Use request model",
        reasoning: "Reasoning",
        useRequestReasoning: "Use request value",
        forceModel: "Force model override",
        forceReasoning: "Force reasoning override",
        save: "Save Policy",
        saving: "Saving...",
        saved: "Policy saved",
        noUsers: "No gateway users",
        alerts: "Alerts",
        alertsSub: "Low quota and protected account timeline",
        noAlerts: "No alerts",
        lowQuotaDetected: (id: string) => `Low quota detected for account ${id}`,
        alertDescription: (label: string, severity: string, level: number) =>
          `${label} entered ${severity.toLowerCase()} state with cooldown level ${level}.`,
        requestLogs: "Request Logs",
        requestLogsSub: "Live requests, user, model, and spend estimate",
        noLogs: "No request logs",
        method: "Method",
        endpoint: "Endpoint",
        user: "User",
        model: "Model",
        cost: "Cost",
        systemConfig: "System Config",
        configSub: "Theme, protection policy, and service health",
        protectionMode: "Protection Mode",
        protectionModeDesc:
          "Automatically move accounts into protected routing when quota runs low.",
        autoRefill: "Auto-refill",
        autoRefillDesc:
          "Try automatic refill or fallback switching in low-quota situations.",
        storage: "Storage",
        services: "Services",
        online: "online",
        offline: "offline",
        theme: "Theme",
        darkTheme: "Dark",
        lightTheme: "Light",
        drawerKicker: "Pool Actions",
        drawerTitle: "Quick Actions",
        syncSnapshot: "Accounts",
        reviewProtected: "User Policy",
        priorityAccounts: "Priority accounts",
        noPriorityAccounts: "No priority accounts",
        language: "Language",
        sidebarState: "Sidebar",
        collapse: "Collapse",
        expand: "Expand",
        systemReady: "System Ready",
        checkSystem: "Check System",
        accountsInPool: (count: number) => `${count} accounts in pool`,
        userCreated: (name: string) => `${name} created`,
        userSaved: "User policy updated",
        refreshFailed: "Failed to refresh models",
        saveFailed: "Failed to save user",
        requestModelHelp: "Override the downstream model",
        requestReasoningHelp: "Override the downstream reasoning effort"
      };
}

function UserPolicyCard({
  availableModels,
  compact = false,
  gatewayBaseUrl,
  language,
  latestIssuedToken,
  onSave,
  recentInstances,
  saving,
  tokenUsage = 0,
  theme,
  user
}: UserPolicyCardProps) {
  const t = translationFor(language);
  const isDark = theme === "dark";
  const [name, setName] = useState(user.name);
  const [email, setEmail] = useState(user.email);
  const [role, setRole] = useState<GatewayUserRole>(user.role);
  const [defaultModel, setDefaultModel] = useState(user.defaultModel ?? "");
  const [reasoningEffort, setReasoningEffort] = useState(
    user.reasoningEffort ?? ""
  );
  const [forceModelOverride, setForceModelOverride] = useState(
    user.forceModelOverride
  );
  const [forceReasoningEffort, setForceReasoningEffort] = useState(
    user.forceReasoningEffort
  );
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [copiedField, setCopiedField] = useState("");

  useEffect(() => {
    setName(user.name);
    setEmail(user.email);
    setRole(user.role);
    setDefaultModel(user.defaultModel ?? "");
    setReasoningEffort(user.reasoningEffort ?? "");
    setForceModelOverride(user.forceModelOverride);
    setForceReasoningEffort(user.forceReasoningEffort);
    setNotice("");
    setError("");
  }, [user]);

  const dirty =
    name !== user.name ||
    email !== user.email ||
    role !== user.role ||
    defaultModel !== (user.defaultModel ?? "") ||
    reasoningEffort !== (user.reasoningEffort ?? "") ||
    forceModelOverride !== user.forceModelOverride ||
    forceReasoningEffort !== user.forceReasoningEffort;

  const affinityValue = `${user.name.toLowerCase().replace(/\s+/g, "-")}-mbp-main`;
  const shellSnippet = `export OPENAI_API_BASE="${gatewayBaseUrl}"\nexport OPENAI_API_KEY="${latestIssuedToken ?? "<gateway-key>"}"\nexport CODEX_AFFINITY_ID="${affinityValue}"`;
  const codexSnippet = `model_provider = "gateway"\nmodel = "${defaultModel || "gpt-5.4"}"\n\n[model_providers.gateway]\nname = "AI Pool Gateway"\nbase_url = "${gatewayBaseUrl}"\nenv_key = "OPENAI_API_KEY"\nenv_http_headers = { "x-codex-cli-affinity-id" = "CODEX_AFFINITY_ID" }`;
  const curlSnippet = `curl ${gatewayBaseUrl.replace(/\/v1$/, "")}/v1/models -H "Authorization: Bearer ${latestIssuedToken ?? "<gateway-key>"}" -H "x-codex-cli-affinity-id: ${affinityValue}"`;
  const routeModeLabel = (value: "direct" | "warp") =>
    value === "warp" ? "Warp" : language === "zh" ? "直连" : "Direct";
  const panelClass = cx(
    "rounded-[30px] border p-5 shadow-soft backdrop-blur-xl",
    isDark
      ? "border-white/10 bg-[linear-gradient(180deg,rgba(255,255,255,0.05),rgba(255,255,255,0.025))]"
      : "border-white/80 bg-[linear-gradient(180deg,rgba(255,255,255,0.98),rgba(244,244,247,0.92))]"
  );
  const inputClass = cx(
    "w-full rounded-[22px] border px-4 py-3 text-sm outline-none transition-all duration-200",
    isDark
      ? "border-white/10 bg-white/[0.04] text-zinc-100 focus:border-sky-400"
      : "border-zinc-200 bg-white text-zinc-700 focus:border-sky-300"
  );
  const readOnlyCardClass = cx(
    "rounded-[24px] border px-4 py-4",
    isDark ? "border-white/10 bg-[#0d1016]" : "border-zinc-200 bg-white"
  );
  const sectionEyebrowClass = cx(
    "text-[11px] uppercase tracking-[0.2em]",
    isDark ? "text-zinc-500" : "text-zinc-400"
  );
  const recentInstanceList = recentInstances.slice(0, compact ? 4 : 6);

  async function handleCopy(value: string, field: string) {
    try {
      await copyText(value);
      setCopiedField(field);
      window.setTimeout(() => {
        setCopiedField((current) => (current === field ? "" : current));
      }, 1200);
    } catch {
      // ignore clipboard failures
    }
  }

  async function handleSave() {
    try {
      setError("");
      setNotice("");
      await onSave(user.id, {
        name,
        email,
        role,
        defaultModel: defaultModel || null,
        reasoningEffort: reasoningEffort || null,
        forceModelOverride,
        forceReasoningEffort
      });
      setNotice(t.saved);
    } catch (saveError) {
      setError(
        saveError instanceof Error ? saveError.message : t.saveFailed
      );
    }
  }

  return (
    <article
      className={cx(
        "rounded-[30px] border p-5 shadow-soft backdrop-blur-xl",
        isDark
          ? "border-white/10 bg-[linear-gradient(180deg,rgba(17,20,27,0.96),rgba(10,12,18,0.92))]"
          : "border-white/80 bg-[linear-gradient(180deg,rgba(255,255,255,0.98),rgba(244,244,247,0.94))]"
      )}
    >
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex items-center gap-3">
          <span
            className={cx(
              "flex h-12 w-12 items-center justify-center rounded-full shadow-soft",
              isDark ? "bg-white/[0.06] text-zinc-200" : "bg-white text-zinc-500"
            )}
          >
            <User size={18} strokeWidth={iconStroke} />
          </span>
          <div>
            <p className={cx("text-sm font-medium", isDark ? "text-zinc-50" : "text-zinc-900")}>
              {user.name}
            </p>
            <p className={cx("mt-1 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
              {user.email}
            </p>
          </div>
        </div>
        <span
          className={cx(
            "rounded-full px-3 py-1.5 text-xs font-medium",
            user.role === "admin"
              ? isDark
                ? "bg-sky-500/14 text-sky-200"
                : "bg-sky-100 text-sky-700"
              : isDark
                ? "bg-white/[0.06] text-zinc-300"
                : "bg-zinc-200 text-zinc-600"
          )}
        >
          {user.role === "admin" ? t.admin : t.viewer}
        </span>
      </div>

      <div
        className={cx(
          "mt-4 grid gap-3",
          compact ? "sm:grid-cols-2" : "md:grid-cols-4"
        )}
      >
        {[
          {
            icon: DollarSign,
            label: t.estimatedSpend,
            value: formatUsd(user.estimatedSpendUsd, language)
          },
          {
            icon: Sparkles,
            label: t.tokens,
            value: formatNumber(tokenUsage, language)
          },
          {
            icon: Activity,
            label: t.requestCount,
            value: formatNumber(user.requestCount, language)
          },
          {
            icon: Clock3,
            label: t.lastUsed,
            value: relativeTime(user.lastUsedAt, language)
          }
        ].map((item) => {
          const Icon = item.icon;
          return (
            <div
              className={cx(
                "rounded-[22px] border px-4 py-3",
                isDark ? "border-white/10 bg-[#0d1016]" : "border-zinc-200 bg-white"
              )}
              key={item.label}
            >
              <p className={cx("flex items-center gap-2 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                <Icon size={14} strokeWidth={iconStroke} />
                {item.label}
              </p>
              <p className={cx("mt-2 text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                {item.value}
              </p>
            </div>
          );
        })}
      </div>

      <div className="mt-5 space-y-4">
        <section className={panelClass}>
          <div className="flex items-start gap-3">
            <span
              className={cx(
                "flex h-11 w-11 items-center justify-center rounded-[20px]",
                isDark ? "bg-white/[0.06] text-zinc-200" : "bg-white text-zinc-600"
              )}
            >
              <User size={18} strokeWidth={iconStroke} />
            </span>
            <div>
              <p className={sectionEyebrowClass}>{t.policySectionKicker}</p>
              <h4
                className={cx(
                  "mt-2 text-lg font-semibold tracking-tight",
                  isDark ? "text-zinc-50" : "text-zinc-900"
                )}
              >
                {t.policySectionTitle}
              </h4>
              <p
                className={cx(
                  "mt-2 text-sm leading-6",
                  isDark ? "text-zinc-400" : "text-zinc-500"
                )}
              >
                {t.policySectionDesc}
              </p>
            </div>
          </div>

          <div className="mt-5 grid gap-4 md:grid-cols-2">
            <div className="space-y-4">
              <label className="block">
                <span className={cx("mb-2 block text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {language === "zh" ? "姓名" : "Name"}
                </span>
                <input
                  className={inputClass}
                  onChange={(event) => setName(event.target.value)}
                  type="text"
                  value={name}
                />
              </label>

              <label className="block">
                <span className={cx("mb-2 block text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {language === "zh" ? "邮箱" : "Email"}
                </span>
                <input
                  className={inputClass}
                  onChange={(event) => setEmail(event.target.value)}
                  type="email"
                  value={email}
                />
              </label>

              <div>
                <span className={cx("mb-2 block text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {language === "zh" ? "角色" : "Role"}
                </span>
                <div className="grid grid-cols-2 gap-3">
                  {[
                    { id: "admin" as const, label: t.admin },
                    { id: "viewer" as const, label: t.viewer }
                  ].map((item) => (
                    <button
                      className={cx(
                        "rounded-[20px] px-4 py-3 text-sm font-medium transition-all duration-200",
                        role === item.id
                          ? isDark
                            ? "bg-zinc-100 text-zinc-950"
                            : "bg-zinc-900 text-white"
                          : isDark
                            ? "bg-white/[0.05] text-zinc-300 hover:bg-white/[0.08]"
                            : "bg-white text-zinc-600 hover:bg-zinc-100"
                      )}
                      key={item.id}
                      onClick={() => setRole(item.id)}
                      type="button"
                    >
                      {item.label}
                    </button>
                  ))}
                </div>
              </div>
            </div>

            <div className="space-y-4">
              <label className="block">
                <span className={cx("mb-2 block text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {t.defaultModel}
                </span>
                <select
                  className={inputClass}
                  onChange={(event) => setDefaultModel(event.target.value)}
                  value={defaultModel}
                >
                  <option value="">{t.useRequestModel}</option>
                  {availableModels.map((model) => (
                    <option key={model} value={model}>
                      {model}
                    </option>
                  ))}
                </select>
              </label>

              <label className="block">
                <span className={cx("mb-2 block text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {t.reasoning}
                </span>
                <select
                  className={inputClass}
                  onChange={(event) => setReasoningEffort(event.target.value)}
                  value={reasoningEffort}
                >
                  <option value="">{t.useRequestReasoning}</option>
                  {["low", "medium", "high", "xhigh"].map((level) => (
                    <option key={level} value={level}>
                      {level}
                    </option>
                  ))}
                </select>
              </label>

              <div
                className={cx(
                  "space-y-3 rounded-[24px] border border-dashed px-4 py-3",
                  isDark ? "border-white/10 bg-white/[0.02]" : "border-zinc-200 bg-white/80"
                )}
              >
                {[
                  {
                    checked: forceModelOverride,
                    description: t.requestModelHelp,
                    label: t.forceModel,
                    onClick: () => setForceModelOverride((value) => !value)
                  },
                  {
                    checked: forceReasoningEffort,
                    description: t.requestReasoningHelp,
                    label: t.forceReasoning,
                    onClick: () => setForceReasoningEffort((value) => !value)
                  }
                ].map((item) => (
                  <button
                    className="flex w-full items-center justify-between gap-4 text-left"
                    key={item.label}
                    onClick={item.onClick}
                    type="button"
                  >
                    <div>
                      <p className={cx("text-sm", isDark ? "text-zinc-200" : "text-zinc-700")}>
                        {item.label}
                      </p>
                      <p className={cx("mt-1 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                        {item.description}
                      </p>
                    </div>
                    <span
                      className={cx(
                        "relative h-7 w-12 rounded-full transition-all duration-200",
                        item.checked
                          ? "bg-sky-500"
                          : isDark
                            ? "bg-white/10"
                            : "bg-zinc-300"
                      )}
                    >
                      <span
                        className={cx(
                          "absolute top-1 h-5 w-5 rounded-full bg-white transition-all duration-200",
                          item.checked ? "left-6" : "left-1"
                        )}
                      />
                    </span>
                  </button>
                ))}
              </div>
            </div>
          </div>

          <div className="mt-5 flex justify-end">
            <button
              className={cx(
                "inline-flex items-center gap-2 rounded-2xl px-4 py-3 text-sm font-medium transition-all duration-200",
                isDark
                  ? "bg-zinc-100 text-zinc-950 hover:opacity-90"
                  : "bg-zinc-900 text-white hover:opacity-90",
                (!dirty || saving) && "opacity-70"
              )}
              disabled={!dirty || saving}
              onClick={handleSave}
              type="button"
            >
              {saving ? (
                <RefreshCw className="animate-spin" size={16} strokeWidth={iconStroke} />
              ) : (
                <CheckCircle2 size={16} strokeWidth={iconStroke} />
              )}
              {saving ? t.saving : t.save}
            </button>
          </div>
        </section>

        <section className={panelClass}>
          <div className="flex items-start gap-3">
            <span
              className={cx(
                "flex h-11 w-11 items-center justify-center rounded-[20px]",
                isDark ? "bg-sky-500/14 text-sky-200" : "bg-sky-50 text-sky-600"
              )}
            >
              <Terminal size={18} strokeWidth={iconStroke} />
            </span>
            <div>
              <p className={sectionEyebrowClass}>{t.accessSectionKicker}</p>
              <h4
                className={cx(
                  "mt-2 text-lg font-semibold tracking-tight",
                  isDark ? "text-zinc-50" : "text-zinc-900"
                )}
              >
                {t.accessSectionTitle}
              </h4>
              <p
                className={cx(
                  "mt-2 text-sm leading-6",
                  isDark ? "text-zinc-400" : "text-zinc-500"
                )}
              >
                {t.accessSectionDesc}
              </p>
            </div>
          </div>

          <div
            className={cx(
              "mt-5 grid gap-4",
              compact ? "grid-cols-1" : "lg:grid-cols-[0.95fr_1.05fr]"
            )}
          >
            <div className="space-y-4">
              <div className="grid gap-3 sm:grid-cols-2">
                {[
                  {
                    icon: Sparkles,
                    label: t.gatewayBase,
                    value: gatewayBaseUrl,
                    field: "base"
                  },
                  {
                    icon: KeyRound,
                    label: t.gatewayKey,
                    value: latestIssuedToken ?? user.tokenPreview,
                    field: "base-key"
                  },
                  {
                    icon: Shield,
                    label: t.affinityHeader,
                    value: "x-codex-cli-affinity-id",
                    field: "affinity-header"
                  },
                  {
                    icon: Activity,
                    label: t.affinityId,
                    value: affinityValue,
                    field: "affinity"
                  }
                ].map((item) => {
                  const Icon = item.icon;
                  return (
                    <div className={readOnlyCardClass} key={item.field}>
                      <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0">
                          <p className={cx("flex items-center gap-2 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                            <Icon size={14} strokeWidth={iconStroke} />
                            {item.label}
                          </p>
                          <p
                            className={cx(
                              "mt-2 break-all text-sm font-medium",
                              isDark ? "text-zinc-100" : "text-zinc-900"
                            )}
                          >
                            {item.value}
                          </p>
                        </div>
                        <button
                          className={cx(
                            "inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-full transition-all duration-200",
                            isDark
                              ? "bg-white/6 text-zinc-300 hover:bg-white/10"
                              : "bg-zinc-100 text-zinc-600 hover:bg-zinc-200"
                          )}
                          onClick={() => handleCopy(item.value, item.field)}
                          title={copiedField === item.field ? t.copied : t.copy}
                          type="button"
                        >
                          <Copy size={14} strokeWidth={iconStroke} />
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>

              <div
                className={cx(
                  "rounded-[24px] border px-4 py-4",
                  isDark ? "border-white/10 bg-white/[0.03]" : "border-zinc-200 bg-zinc-50"
                )}
              >
                <p className={cx("text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                  {t.multiCliTitle}
                </p>
                <p className={cx("mt-2 text-xs leading-6", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {t.multiCliDesc}
                </p>
                <p className={cx("mt-3 text-xs leading-6", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {t.affinityHint}
                </p>
                {!latestIssuedToken ? (
                  <p className={cx("mt-3 text-xs leading-6", isDark ? "text-zinc-500" : "text-zinc-500")}>
                    {t.fullKeyOnce}
                  </p>
                ) : null}
              </div>
            </div>

            <div className="space-y-4">
              {[
                {
                  field: "shell",
                  label: t.envPreset,
                  value: shellSnippet,
                  icon: Terminal
                },
                {
                  field: "codex",
                  label: t.codexConfig,
                  value: codexSnippet,
                  icon: Bot
                },
                {
                  field: "curl",
                  label: t.curlExample,
                  value: curlSnippet,
                  icon: Sparkles
                }
              ].map((item) => {
                const Icon = item.icon;
                return (
                  <div
                    className={cx(
                      "rounded-[24px] border p-4",
                      isDark ? "border-white/10 bg-[#0b0d12]" : "border-zinc-200 bg-zinc-950"
                    )}
                    key={item.field}
                  >
                    <div className="mb-3 flex items-center justify-between gap-3">
                      <p className="flex items-center gap-2 text-xs uppercase tracking-[0.18em] text-zinc-500">
                        <Icon size={14} strokeWidth={iconStroke} />
                        {item.label}
                      </p>
                      <button
                        className={cx(
                          "inline-flex h-9 w-9 items-center justify-center rounded-full text-xs transition-all duration-200",
                          isDark
                            ? "bg-white/6 text-zinc-300 hover:bg-white/10"
                            : "bg-white/10 text-zinc-100 hover:bg-white/15"
                        )}
                        onClick={() => handleCopy(item.value, item.field)}
                        title={copiedField === item.field ? t.copied : t.copy}
                        type="button"
                      >
                        <Copy size={14} strokeWidth={iconStroke} />
                      </button>
                    </div>
                    <pre className="overflow-auto text-xs leading-6 text-zinc-200">
                      <code>{item.value}</code>
                    </pre>
                  </div>
                );
              })}
            </div>
          </div>
        </section>

        <section className={panelClass}>
          <div className="flex items-start gap-3">
            <span
              className={cx(
                "flex h-11 w-11 items-center justify-center rounded-[20px]",
                isDark ? "bg-emerald-500/14 text-emerald-200" : "bg-emerald-50 text-emerald-600"
              )}
            >
              <Activity size={18} strokeWidth={iconStroke} />
            </span>
            <div>
              <p className={sectionEyebrowClass}>{t.instancesSectionKicker}</p>
              <h4
                className={cx(
                  "mt-2 text-lg font-semibold tracking-tight",
                  isDark ? "text-zinc-50" : "text-zinc-900"
                )}
              >
                {t.instancesSectionTitle}
              </h4>
              <p
                className={cx(
                  "mt-2 text-sm leading-6",
                  isDark ? "text-zinc-400" : "text-zinc-500"
                )}
              >
                {t.instancesSectionDesc}
              </p>
            </div>
          </div>

          {recentInstanceList.length === 0 ? (
            <div
              className={cx(
                "mt-5 flex min-h-[160px] items-center justify-center rounded-[24px] border border-dashed text-center",
                isDark
                  ? "border-white/10 bg-white/[0.02] text-zinc-500"
                  : "border-zinc-200 bg-zinc-50 text-zinc-400"
              )}
            >
              <p className="max-w-sm text-sm leading-7">{t.noRecentInstances}</p>
            </div>
          ) : (
            <div className="mt-5 space-y-3">
              {recentInstanceList.map((instance) => (
                <div className={readOnlyCardClass} key={instance.principalId}>
                  <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
                    <div className="min-w-0 flex-1">
                      <p className={sectionEyebrowClass}>{t.cliInstance}</p>
                      <div className="mt-2 flex flex-wrap items-center gap-2">
                        <span
                          className={cx(
                            "inline-flex max-w-full items-center rounded-full px-3 py-1.5 text-sm font-medium",
                            isDark ? "bg-white/[0.06] text-zinc-100" : "bg-zinc-100 text-zinc-700"
                          )}
                        >
                          <span className="truncate font-mono">{instance.affinityId}</span>
                        </span>
                        {instance.activeLease ? (
                          <span
                            className={cx(
                              "rounded-full px-3 py-1.5 text-xs font-medium",
                              isDark ? "bg-emerald-500/14 text-emerald-200" : "bg-emerald-50 text-emerald-700"
                            )}
                          >
                            {t.activeLease}
                            {instance.activeSubagents > 0
                              ? ` · ${instance.activeSubagents} ${t.subagents}`
                              : ""}
                          </span>
                        ) : null}
                        <span
                          className={cx(
                            "rounded-full px-3 py-1.5 text-xs font-medium",
                            instance.statusCode >= 400
                              ? isDark
                                ? "bg-rose-500/14 text-rose-200"
                                : "bg-rose-50 text-rose-700"
                              : isDark
                                ? "bg-sky-500/14 text-sky-200"
                                : "bg-sky-50 text-sky-700"
                          )}
                        >
                          {t.statusCode} {instance.statusCode}
                        </span>
                      </div>
                      <p
                        className={cx(
                          "mt-3 truncate font-mono text-xs",
                          isDark ? "text-zinc-500" : "text-zinc-500"
                        )}
                      >
                        {instance.principalId}
                      </p>
                    </div>

                    <div className="shrink-0 text-left sm:text-right">
                      <p className={cx("text-sm font-medium", isDark ? "text-zinc-50" : "text-zinc-900")}>
                        {relativeTime(instance.lastUsedAt, language)}
                      </p>
                      <p className={cx("mt-1 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                        {formatDateTime(instance.lastUsedAt, language)}
                      </p>
                    </div>
                  </div>

                  <div className="mt-4 grid gap-3 sm:grid-cols-4">
                    {[
                      {
                        label: t.requestCount,
                        value: formatNumber(instance.requestCount, language)
                      },
                      {
                        label: t.tokens,
                        value: formatNumber(instance.totalTokens, language)
                      },
                      {
                        label: t.model,
                        value: instance.lastModel
                      },
                      {
                        label: t.route,
                        value: routeModeLabel(instance.routeMode)
                      }
                    ].map((item) => (
                      <div
                        className={cx(
                          "rounded-[20px] border px-3 py-3",
                          isDark ? "border-white/10 bg-white/[0.03]" : "border-zinc-200 bg-zinc-50"
                        )}
                        key={item.label}
                      >
                        <p className={cx("text-[11px] uppercase tracking-[0.15em]", isDark ? "text-zinc-500" : "text-zinc-400")}>
                          {item.label}
                        </p>
                        <p className={cx("mt-2 text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                          {item.value}
                        </p>
                      </div>
                    ))}
                  </div>

                  <div className="mt-4 grid gap-3 sm:grid-cols-2">
                    {[
                      {
                        label: t.servedBy,
                        value: instance.lastAccountLabel
                      },
                      {
                        label: t.lastPath,
                        value: instance.lastEndpoint
                      }
                    ].map((item) => (
                      <div
                        className={cx(
                          "rounded-[20px] border px-4 py-3",
                          isDark ? "border-white/10 bg-white/[0.03]" : "border-zinc-200 bg-zinc-50"
                        )}
                        key={item.label}
                      >
                        <p className={cx("text-[11px] uppercase tracking-[0.15em]", isDark ? "text-zinc-500" : "text-zinc-400")}>
                          {item.label}
                        </p>
                        <p className={cx("mt-2 break-all text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                          {item.value}
                        </p>
                      </div>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          )}
        </section>
      </div>

      {notice ? (
        <div
          className={cx(
            "mt-4 rounded-[20px] px-4 py-3 text-sm",
            isDark ? "bg-emerald-500/10 text-emerald-200" : "bg-emerald-50 text-emerald-700"
          )}
        >
          {notice}
        </div>
      ) : null}

      {error ? (
        <div
          className={cx(
            "mt-4 rounded-[20px] px-4 py-3 text-sm",
            isDark ? "bg-rose-500/10 text-rose-200" : "bg-rose-50 text-rose-700"
          )}
        >
          {error}
        </div>
      ) : null}
    </article>
  );
}

export function DashboardApp({
  snapshot,
  health,
  initialLanguage = "zh",
  callbackUrl,
  gatewayBaseUrl
}: DashboardAppProps) {
  const [language, setLanguage] = useState<Language>(initialLanguage);
  const [theme, setTheme] = useState<ThemeMode>("dark");
  const [runtimeSnapshot, setRuntimeSnapshot] = useState(snapshot);
  const [sidebarExpanded, setSidebarExpanded] = useState(false);
  const [sidebarHovered, setSidebarHovered] = useState(false);
  const [activeView, setActiveView] = useState<ActiveView>("overview");
  const [isDrawerOpen, setIsDrawerOpen] = useState(false);
  const [isAccountDrawerOpen, setIsAccountDrawerOpen] = useState(false);
  const [isUserModalOpen, setIsUserModalOpen] = useState(false);
  const [selectedUserId, setSelectedUserId] = useState<string | null>(null);
  const [accountFilter, setAccountFilter] = useState<AccountFilter>("all");
  const [protectionMode, setProtectionMode] = useState(true);
  const [autoRefill, setAutoRefill] = useState(false);
  const [refreshingModels, setRefreshingModels] = useState(false);
  const [savingUserId, setSavingUserId] = useState<string | null>(null);
  const [banner, setBanner] = useState<BannerState>(null);
  const [latestCreatedUser, setLatestCreatedUser] =
    useState<CreatedGatewayUser | null>(null);

  const t = translationFor(language);
  const isDark = theme === "dark";
  const effectiveSidebarExpanded = sidebarExpanded || sidebarHovered;

  useEffect(() => {
    setRuntimeSnapshot(snapshot);
  }, [snapshot]);

  useEffect(() => {
    try {
      const storedTheme = window.localStorage.getItem(themeStorageKey);
      if (storedTheme === "dark" || storedTheme === "light") {
        setTheme(storedTheme);
      }
      const storedLanguage = window.localStorage.getItem(languageStorageKey);
      if (storedLanguage === "zh" || storedLanguage === "en") {
        setLanguage(storedLanguage);
      }
    } catch {
      // ignore storage failures
    }
  }, []);

  useEffect(() => {
    try {
      window.localStorage.setItem(themeStorageKey, theme);
    } catch {
      // ignore storage failures
    }

    document.body.classList.remove("theme-dark", "theme-light");
    document.body.classList.add(theme === "dark" ? "theme-dark" : "theme-light");
    document.documentElement.style.colorScheme = theme;

    return () => {
      document.body.classList.remove("theme-dark", "theme-light");
      document.documentElement.style.colorScheme = "";
    };
  }, [theme]);

  useEffect(() => {
    try {
      window.localStorage.setItem(languageStorageKey, language);
    } catch {
      // ignore storage failures
    }
  }, [language]);

  useEffect(() => {
    if (!banner) {
      return undefined;
    }

    const timer = window.setTimeout(() => setBanner(null), 2800);
    return () => window.clearTimeout(timer);
  }, [banner]);

  useEffect(() => {
    if (activeView !== "users" && selectedUserId) {
      setSelectedUserId(null);
    }
  }, [activeView, selectedUserId]);

  const lowQuotaAccounts = useMemo(
    () =>
      runtimeSnapshot.accounts.filter((account) => accountKind(account) === "low"),
    [runtimeSnapshot.accounts]
  );
  const activeAccounts = useMemo(
    () =>
      runtimeSnapshot.accounts.filter((account) => accountKind(account) === "active"),
    [runtimeSnapshot.accounts]
  );
  const protectedAccounts = useMemo(
    () =>
      runtimeSnapshot.accounts.filter(
        (account) => accountKind(account) === "protected"
      ),
    [runtimeSnapshot.accounts]
  );

  const filteredAccounts = useMemo(() => {
    if (accountFilter === "all") {
      return runtimeSnapshot.accounts;
    }

    return runtimeSnapshot.accounts.filter((account) => {
      const kind = accountKind(account);
      if (accountFilter === "active") {
        return kind === "active";
      }
      if (accountFilter === "low") {
        return kind === "low";
      }
      return kind === "protected";
    });
  }, [accountFilter, runtimeSnapshot.accounts]);

  const requestSeries = useMemo(
    () =>
      buildRequestSeries(
        runtimeSnapshot.requestLogs,
        Math.max(
          runtimeSnapshot.counts.accounts * 16,
          runtimeSnapshot.counts.activeLeases * 24,
          28
        )
      ),
    [runtimeSnapshot.counts.accounts, runtimeSnapshot.counts.activeLeases, runtimeSnapshot.requestLogs]
  );

  const availableModels = useMemo(
    () => deriveModelCatalog(runtimeSnapshot),
    [runtimeSnapshot]
  );

  const topSpendUsers = useMemo(
    () =>
      [...runtimeSnapshot.users]
        .sort(
          (left, right) => right.estimatedSpendUsd - left.estimatedSpendUsd
        )
        .slice(0, 4),
    [runtimeSnapshot.users]
  );

  const userActivityByEmail = useMemo(() => {
    const leaseByPrincipal = new Map(
      runtimeSnapshot.leases.map((lease) => [lease.principalId, lease])
    );
    const grouped = new Map<
      string,
      {
        totalTokens: number;
        recentInstances: Map<
          string,
          Omit<
            RecentCliInstance,
            "activeLease" | "activeSubagents"
          > & { lastUsedAtMs: number }
        >;
      }
    >();

    for (const log of runtimeSnapshot.requestLogs) {
      const emailKey = log.userEmail.trim().toLowerCase();
      if (!emailKey) {
        continue;
      }

      const createdAtMs = new Date(log.createdAt).getTime();
      const entry = grouped.get(emailKey) ?? {
        totalTokens: 0,
        recentInstances: new Map()
      };
      entry.totalTokens += log.usage.totalTokens;

      const existing = entry.recentInstances.get(log.principalId);
      if (!existing) {
        entry.recentInstances.set(log.principalId, {
          principalId: log.principalId,
          affinityId: affinityIdFromPrincipal(log.principalId),
          requestCount: 1,
          totalTokens: log.usage.totalTokens,
          estimatedSpendUsd: log.estimatedCostUsd ?? 0,
          lastUsedAt: log.createdAt,
          lastUsedAtMs: createdAtMs,
          lastEndpoint: log.endpoint,
          lastModel: log.effectiveModel,
          lastAccountLabel: log.accountLabel,
          routeMode: log.routeMode,
          statusCode: log.statusCode
        });
      } else {
        existing.requestCount += 1;
        existing.totalTokens += log.usage.totalTokens;
        existing.estimatedSpendUsd += log.estimatedCostUsd ?? 0;
        if (createdAtMs >= existing.lastUsedAtMs) {
          existing.lastUsedAt = log.createdAt;
          existing.lastUsedAtMs = createdAtMs;
          existing.lastEndpoint = log.endpoint;
          existing.lastModel = log.effectiveModel;
          existing.lastAccountLabel = log.accountLabel;
          existing.routeMode = log.routeMode;
          existing.statusCode = log.statusCode;
        }
      }

      grouped.set(emailKey, entry);
    }

    const activity = new Map<
      string,
      {
        totalTokens: number;
        recentInstances: RecentCliInstance[];
      }
    >();

    for (const [emailKey, entry] of grouped) {
      const recentInstances = Array.from(entry.recentInstances.values())
        .sort((left, right) => right.lastUsedAtMs - left.lastUsedAtMs)
        .slice(0, 6)
        .map(({ lastUsedAtMs: _lastUsedAtMs, ...instance }) => {
          const lease = leaseByPrincipal.get(instance.principalId);
          return {
            ...instance,
            activeLease: Boolean(lease),
            activeSubagents: lease?.activeSubagents ?? 0
          };
        });

      activity.set(emailKey, {
        totalTokens: entry.totalTokens,
        recentInstances
      });
    }

    return activity;
  }, [runtimeSnapshot.leases, runtimeSnapshot.requestLogs]);

  const usersWithUsage = useMemo<UserUsageEntry[]>(
    () =>
      runtimeSnapshot.users
        .map((user) => {
          const activity = userActivityByEmail.get(user.email.trim().toLowerCase());
          return {
            user,
            totalTokens: activity?.totalTokens ?? 0,
            recentInstances: activity?.recentInstances ?? []
          };
        })
        .sort((left, right) => {
          if (right.user.estimatedSpendUsd !== left.user.estimatedSpendUsd) {
            return right.user.estimatedSpendUsd - left.user.estimatedSpendUsd;
          }
          if (right.totalTokens !== left.totalTokens) {
            return right.totalTokens - left.totalTokens;
          }
          const leftLastUsed = left.user.lastUsedAt
            ? new Date(left.user.lastUsedAt).getTime()
            : 0;
          const rightLastUsed = right.user.lastUsedAt
            ? new Date(right.user.lastUsedAt).getTime()
            : 0;
          return rightLastUsed - leftLastUsed;
        }),
    [runtimeSnapshot.users, userActivityByEmail]
  );

  const activeUserCount = useMemo(
    () =>
      usersWithUsage.filter(
        ({ user }) => user.requestCount > 0 || user.lastUsedAt !== null
      ).length,
    [usersWithUsage]
  );

  const selectedUserEntry = useMemo(
    () =>
      selectedUserId
        ? usersWithUsage.find(({ user }) => user.id === selectedUserId) ?? null
        : null,
    [selectedUserId, usersWithUsage]
  );

  const priorityAccounts = useMemo(
    () => [...protectedAccounts, ...lowQuotaAccounts].slice(0, 6),
    [lowQuotaAccounts, protectedAccounts]
  );

  const navItems = [
    { id: "overview" as const, label: t.navOverview, icon: LayoutDashboard },
    { id: "accounts" as const, label: t.navAccounts, icon: Bot },
    { id: "users" as const, label: t.navUsers, icon: Users },
    { id: "alerts" as const, label: t.navAlerts, icon: ShieldAlert },
    { id: "logs" as const, label: t.navLogs, icon: FileText },
    { id: "config" as const, label: t.navConfig, icon: Settings }
  ];

  async function handleRefreshModels() {
    try {
      setRefreshingModels(true);
      const response = await fetch("/api/dashboard/accounts/models/refresh", {
        method: "POST"
      });
      const payload = (await response.json().catch(() => null)) as
        | DashboardSnapshot["accounts"]
        | { error?: { message?: string } }
        | null;

      if (!response.ok || !Array.isArray(payload)) {
        const message =
          payload &&
          typeof payload === "object" &&
          !Array.isArray(payload) &&
          "error" in payload
            ? payload.error?.message
            : undefined;
        throw new Error(message || t.refreshFailed);
      }

      setRuntimeSnapshot((current) => ({
        ...current,
        accounts: payload,
        modelCatalog: Array.from(
          new Set(payload.flatMap((account) => account.models))
        ).sort()
      }));
      setBanner({
        tone: "ok",
        message: t.modelsReady
      });
    } catch (error) {
      setBanner({
        tone: "error",
        message: error instanceof Error ? error.message : t.refreshFailed
      });
    } finally {
      setRefreshingModels(false);
    }
  }

  async function handleSaveUser(
    userId: string,
    payload: {
      name?: string;
      email?: string;
      role?: GatewayUserRole;
      defaultModel?: string | null;
      reasoningEffort?: string | null;
      forceModelOverride?: boolean;
      forceReasoningEffort?: boolean;
    }
  ) {
    setSavingUserId(userId);
    try {
      const response = await fetch(`/api/dashboard/users/${userId}`, {
        method: "PUT",
        headers: {
          "content-type": "application/json"
        },
        body: JSON.stringify(payload)
      });
      const result = (await response.json().catch(() => null)) as
        | GatewayUserView
        | { error?: { message?: string } }
        | null;

      if (!response.ok || !result || Array.isArray(result)) {
        const message =
          result &&
          typeof result === "object" &&
          !Array.isArray(result) &&
          "error" in result
            ? result.error?.message
            : undefined;
        throw new Error(message || t.saveFailed);
      }

      const updatedUser = result as GatewayUserView;
      setRuntimeSnapshot((current) => ({
        ...current,
        users: current.users.map((user) =>
          user.id === updatedUser.id ? updatedUser : user
        )
      }));
      setBanner({
        tone: "ok",
        message: t.userSaved
      });
    } finally {
      setSavingUserId(null);
    }
  }

  function handleUserCreated(created: CreatedGatewayUser) {
    setLatestCreatedUser(created);
    setSelectedUserId(created.user.id);
    setRuntimeSnapshot((current) => {
      const existing = current.users.filter((user) => user.id !== created.user.id);
      return {
        ...current,
        users: [created.user, ...existing],
        counts: {
          ...current.counts,
          users: existing.length + 1
        }
      };
    });
    setBanner({
      tone: "ok",
      message: t.userCreated(created.user.name)
    });
  }

  const renderOverview = () => (
    <div className="space-y-6">
      <SectionCard
        actions={
          <button
            className={cx(
              "flex h-11 w-11 items-center justify-center rounded-2xl text-white shadow-soft transition-all duration-200 hover:opacity-90",
              isDark ? "bg-sky-500" : "bg-zinc-900"
            )}
            onClick={() => setIsDrawerOpen(true)}
            type="button"
          >
            <Plus size={18} strokeWidth={iconStroke} />
          </button>
        }
        icon={LayoutDashboard}
        subtitle={t.overviewSub}
        theme={theme}
        title={t.overview}
      >
        <div className="grid gap-4 xl:grid-cols-5">
          <div
            className={cx(
              "rounded-[28px] p-5",
              isDark ? "bg-[#0c0f15]" : "bg-zinc-50"
            )}
          >
            <div className="mb-4 flex items-center justify-between">
              <div
                className={cx(
                  "flex h-9 w-9 items-center justify-center rounded-2xl shadow-soft",
                  isDark ? "bg-white/[0.05] text-sky-200" : "bg-white text-sky-600"
                )}
              >
                <Database size={18} strokeWidth={iconStroke} />
              </div>
              <span className={cx("text-xs font-medium", isDark ? "text-zinc-500" : "text-zinc-400")}>
                {t.cacheHit}
              </span>
            </div>
            <div className="flex items-center gap-4">
              <div className="relative h-24 w-24">
                <svg className="h-24 w-24 -rotate-90" viewBox="0 0 96 96">
                  <circle
                    className={isDark ? "stroke-white/10" : "stroke-zinc-200"}
                    cx="48"
                    cy="48"
                    fill="none"
                    r="38"
                    strokeWidth="10"
                  />
                  <circle
                    className="stroke-sky-500 transition-all duration-500"
                    cx="48"
                    cy="48"
                    fill="none"
                    r="38"
                    strokeLinecap="round"
                    strokeWidth="10"
                    style={donutStyle(runtimeSnapshot.cacheMetrics.prefixHitRatio)}
                  />
                </svg>
                <div className="absolute inset-0 flex items-center justify-center text-center">
                  <div>
                    <p className={cx("text-xl font-semibold", isDark ? "text-zinc-50" : "text-zinc-900")}>
                      {percent(runtimeSnapshot.cacheMetrics.prefixHitRatio)}%
                    </p>
                    <p className={cx("text-[11px]", isDark ? "text-zinc-500" : "text-zinc-400")}>
                      {t.hit}
                    </p>
                  </div>
                </div>
              </div>
              <div className="space-y-1">
                <p className={cx("text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                  {t.prefixCache}
                </p>
                <p className={cx("text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {formatNumber(runtimeSnapshot.cacheMetrics.cachedTokens, language)}{" "}
                  {t.tokens}
                </p>
              </div>
            </div>
          </div>

          {[
            {
              icon: TriangleAlert,
              iconClass: isDark
                ? "bg-amber-500/14 text-amber-200"
                : "bg-amber-50 text-amber-600",
              label: t.lowQuota,
              value: lowQuotaAccounts.length,
              desc: t.lowQuotaDesc
            },
            {
              icon: Activity,
              iconClass: isDark
                ? "bg-emerald-500/14 text-emerald-200"
                : "bg-emerald-50 text-emerald-600",
              label: t.active,
              value: activeAccounts.length,
              desc: t.activeDesc
            },
            {
              icon: Shield,
              iconClass: isDark
                ? "bg-sky-500/14 text-sky-200"
                : "bg-sky-50 text-sky-600",
              label: t.protected,
              value: protectedAccounts.length,
              desc: t.protectedDesc
            },
            {
              icon: DollarSign,
              iconClass: isDark
                ? "bg-violet-500/14 text-violet-200"
                : "bg-violet-50 text-violet-600",
              label: t.totalSpend,
              value: formatUsd(runtimeSnapshot.billing.totalSpendUsd, language),
              desc: t.spendDesc
            }
          ].map((item) => {
            const Icon = item.icon;
            return (
              <div
                className={cx(
                  "rounded-[28px] p-5",
                  isDark ? "bg-[#0c0f15]" : "bg-zinc-50"
                )}
                key={item.label}
              >
                <div className="mb-4 flex items-center justify-between">
                  <div
                    className={cx(
                      "flex h-9 w-9 items-center justify-center rounded-2xl shadow-soft",
                      item.iconClass
                    )}
                  >
                    <Icon size={18} strokeWidth={iconStroke} />
                  </div>
                  <span className={cx("text-xs font-medium", isDark ? "text-zinc-500" : "text-zinc-400")}>
                    {item.label}
                  </span>
                </div>
                <p className={cx("text-[30px] font-semibold tracking-tight", isDark ? "text-zinc-50" : "text-zinc-900")}>
                  {item.value}
                </p>
                <p className={cx("mt-2 text-xs leading-5", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {item.desc}
                </p>
              </div>
            );
          })}
        </div>
      </SectionCard>

      <div className="grid gap-6 xl:grid-cols-[1.55fr_0.85fr]">
        <SectionCard
          icon={GaugeCircle}
          subtitle={t.requestsOverTime}
          theme={theme}
          title={t.requestsOverTime}
        >
          <div
            className={cx(
              "h-[300px] rounded-[28px] p-3",
              isDark ? "bg-[#0c0f15]" : "bg-zinc-50"
            )}
          >
            <ResponsiveContainer height="100%" width="100%">
              <AreaChart data={requestSeries}>
                <defs>
                  <linearGradient id="requestFill" x1="0" x2="0" y1="0" y2="1">
                    <stop offset="5%" stopColor="#38bdf8" stopOpacity={0.42} />
                    <stop offset="95%" stopColor="#38bdf8" stopOpacity={0.03} />
                  </linearGradient>
                </defs>
                <Tooltip
                  contentStyle={{
                    borderRadius: 18,
                    border: isDark
                      ? "1px solid rgba(255,255,255,0.08)"
                      : "1px solid rgba(228,228,231,1)",
                    boxShadow: "0 12px 30px rgba(15,23,42,0.08)",
                    background: isDark
                      ? "rgba(17,20,27,0.96)"
                      : "rgba(255,255,255,0.94)",
                    color: isDark ? "#e4e4e7" : "#18181b",
                    fontSize: 12
                  }}
                  cursor={{
                    stroke: isDark ? "rgba(255,255,255,0.16)" : "#d4d4d8",
                    strokeDasharray: "4 4"
                  }}
                  formatter={(value: number) => [`${value}`, t.requests]}
                  labelStyle={{
                    color: isDark ? "#a1a1aa" : "#71717a",
                    fontSize: 12
                  }}
                />
                <Area
                  dataKey="value"
                  fill="url(#requestFill)"
                  stroke="#38bdf8"
                  strokeWidth={2.5}
                  type="monotone"
                />
              </AreaChart>
            </ResponsiveContainer>
          </div>
        </SectionCard>

        <SectionCard
          icon={Users}
          subtitle={t.topUsers}
          theme={theme}
          title={t.topUsers}
        >
          <div className="space-y-3">
            {topSpendUsers.length === 0 ? (
              <EmptyState icon={Users} theme={theme} title={t.noUsers} />
            ) : (
              topSpendUsers.map((user) => (
                <div
                  className={cx(
                    "flex items-center justify-between rounded-[24px] border px-4 py-3",
                    isDark
                      ? "border-white/10 bg-white/[0.03]"
                      : "border-zinc-200 bg-zinc-50"
                  )}
                  key={user.id}
                >
                  <div className="flex items-center gap-3">
                    <span
                      className={cx(
                        "flex h-10 w-10 items-center justify-center rounded-full",
                        isDark ? "bg-white/[0.06] text-zinc-200" : "bg-white text-zinc-500"
                      )}
                    >
                      <User size={16} strokeWidth={iconStroke} />
                    </span>
                    <div>
                      <p className={cx("text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                        {user.name}
                      </p>
                      <p className={cx("mt-1 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                        {formatNumber(user.requestCount, language)} {t.requests}
                      </p>
                    </div>
                  </div>
                  <div className="text-right">
                    <p className={cx("text-sm font-medium", isDark ? "text-zinc-50" : "text-zinc-900")}>
                      {formatUsd(user.estimatedSpendUsd, language)}
                    </p>
                    <p className={cx("mt-1 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                      {relativeTime(user.lastUsedAt, language)}
                    </p>
                  </div>
                </div>
              ))
            )}

            <div
              className={cx(
                "rounded-[24px] border p-4",
                isDark ? "border-white/10 bg-[#0c0f15]" : "border-zinc-200 bg-zinc-50"
              )}
            >
              <div className="flex items-center justify-between">
                <p className={cx("text-xs uppercase tracking-[0.18em]", isDark ? "text-zinc-500" : "text-zinc-400")}>
                  {t.pricedRequests}
                </p>
                <p className={cx("text-lg font-semibold", isDark ? "text-zinc-50" : "text-zinc-900")}>
                  {formatNumber(runtimeSnapshot.billing.pricedRequests, language)}
                </p>
              </div>
              <p className={cx("mt-3 text-xs leading-5", isDark ? "text-zinc-500" : "text-zinc-500")}>
                {formatNumber(runtimeSnapshot.billing.totalTokens, language)} tokens ·{" "}
                {formatNumber(runtimeSnapshot.billing.totalOutputTokens, language)} output
              </p>
            </div>
          </div>
        </SectionCard>
      </div>
    </div>
  );

  const renderAccounts = () => (
    <SectionCard
      actions={
        <div className="flex flex-wrap items-center gap-3">
          <button
            className={cx(
              "inline-flex items-center gap-2 rounded-2xl px-4 py-2.5 text-sm font-medium transition-all duration-200",
              isDark
                ? "bg-white/[0.06] text-zinc-200 hover:bg-white/[0.1]"
                : "bg-zinc-100 text-zinc-600 hover:bg-zinc-200",
              refreshingModels && "opacity-70"
            )}
            disabled={refreshingModels}
            onClick={handleRefreshModels}
            type="button"
          >
            <RefreshCw
              className={refreshingModels ? "animate-spin" : undefined}
              size={16}
              strokeWidth={iconStroke}
            />
            <span className="hidden sm:inline">
              {refreshingModels ? t.refreshing : t.refreshModels}
            </span>
          </button>
          <button
            className={cx(
              "inline-flex items-center gap-2 rounded-2xl px-4 py-2.5 text-sm font-medium transition-all duration-200 hover:opacity-90",
              isDark ? "bg-zinc-100 text-zinc-950" : "bg-zinc-900 text-white"
            )}
            onClick={() => setIsAccountDrawerOpen(true)}
            type="button"
          >
            <Plus size={16} strokeWidth={iconStroke} />
            <span className="hidden sm:inline">{t.addAccount}</span>
          </button>
        </div>
      }
      icon={Bot}
      subtitle={t.accountsSub}
      theme={theme}
      title={t.accounts}
    >
      <div className="mb-5 flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap gap-2">
          {[
            { id: "all" as const, label: t.all },
            { id: "active" as const, label: t.active },
            { id: "low" as const, label: t.lowQuota },
            { id: "protected" as const, label: t.protected }
          ].map((filter) => (
            <button
              className={cx(
                "rounded-full px-4 py-2 text-xs font-medium transition-all duration-200",
                accountFilter === filter.id
                  ? isDark
                    ? "bg-zinc-100 text-zinc-950"
                    : "bg-zinc-900 text-white"
                  : isDark
                    ? "bg-white/[0.05] text-zinc-400 hover:bg-white/[0.08]"
                    : "bg-zinc-100 text-zinc-500 hover:bg-zinc-200"
              )}
              key={filter.id}
              onClick={() => setAccountFilter(filter.id)}
              type="button"
            >
              {filter.label}
            </button>
          ))}
        </div>

        <div
          className={cx(
            "flex items-center gap-2 rounded-2xl px-3 py-2",
            isDark ? "bg-white/[0.04] text-zinc-500" : "bg-zinc-100 text-zinc-500"
          )}
        >
          <Search size={16} strokeWidth={iconStroke} />
          <span className="text-xs">{t.searchDisabled}</span>
        </div>
      </div>

      <div className="grid gap-4 md:grid-cols-2 2xl:grid-cols-3">
        {filteredAccounts.map((account) => {
          const kind = accountKind(account);
          const quota = Math.min(
            account.quotaHeadroom,
            account.quotaHeadroom5h,
            account.quotaHeadroom7d
          );

          return (
            <article
              className={cx(
                "rounded-[28px] border p-5 shadow-soft",
                isDark
                  ? "border-white/10 bg-white/[0.03]"
                  : "border-zinc-200 bg-zinc-50"
              )}
              key={account.id}
            >
              <div className="mb-4 flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-3">
                    <span
                      className={cx(
                        "h-2.5 w-2.5 rounded-full",
                        kind === "active"
                          ? "bg-emerald-500"
                          : kind === "protected"
                            ? "bg-amber-500"
                            : "bg-rose-500"
                      )}
                    />
                    <p className={cx("truncate text-sm font-medium", isDark ? "text-zinc-50" : "text-zinc-900")}>
                      {account.label || shortId(account.id)}
                    </p>
                  </div>
                  <p className={cx("mt-2 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                    {shortId(account.id)}
                  </p>
                </div>
                <button
                  className={cx(
                    "flex h-9 w-9 items-center justify-center rounded-2xl transition-all duration-200",
                    isDark
                      ? "bg-white/[0.05] text-zinc-400 hover:bg-white/[0.08]"
                      : "bg-white text-zinc-500 hover:bg-zinc-100"
                  )}
                  type="button"
                >
                  <MoreHorizontal size={16} strokeWidth={iconStroke} />
                </button>
              </div>

              <div className="space-y-2">
                <div className={cx("flex items-center justify-between text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  <span>{t.quota}</span>
                  <span>{percent(quota)}%</span>
                </div>
                <div className={cx("h-2 rounded-full", isDark ? "bg-white/10" : "bg-zinc-200")}>
                  <div
                    className={cx("h-2 rounded-full", quotaBarColor(quota))}
                    style={{ width: `${percent(quota)}%` }}
                  />
                </div>
              </div>

              <div className="mt-4 grid grid-cols-3 gap-3">
                {[
                  {
                    icon: ArrowUpRight,
                    label: t.route,
                    value: account.currentMode
                  },
                  {
                    icon: Sparkles,
                    label: t.modelCount,
                    value: String(account.models.length)
                  },
                  {
                    icon: Shield,
                    label: t.protected,
                    value: kind === "protected" ? "on" : "off"
                  }
                ].map((item) => {
                  const Icon = item.icon;
                  return (
                    <div
                      className={cx(
                        "rounded-[20px] border px-3 py-3",
                        isDark ? "border-white/10 bg-[#0c0f15]" : "border-zinc-200 bg-white"
                      )}
                      key={item.label}
                    >
                      <p className={cx("flex items-center gap-2 text-[11px]", isDark ? "text-zinc-500" : "text-zinc-500")}>
                        <Icon size={13} strokeWidth={iconStroke} />
                        {item.label}
                      </p>
                      <p className={cx("mt-2 text-sm font-medium uppercase", isDark ? "text-zinc-100" : "text-zinc-900")}>
                        {item.value}
                      </p>
                    </div>
                  );
                })}
              </div>

              <div className="mt-4 flex flex-wrap gap-2">
                {account.models.slice(0, 4).map((model) => (
                  <span
                    className={cx(
                      "rounded-full px-3 py-1.5 text-[11px]",
                      isDark
                        ? "bg-white/[0.05] text-zinc-300"
                        : "bg-white text-zinc-600"
                    )}
                    key={model}
                  >
                    {model}
                  </span>
                ))}
              </div>
            </article>
          );
        })}
      </div>
    </SectionCard>
  );

  const renderUsers = () => (
    <div className="space-y-6">
      <SectionCard
        actions={
          <div className="flex items-center gap-2">
            <button
              className={cx(
                "inline-flex items-center gap-2 rounded-2xl px-4 py-2.5 text-sm font-medium transition-all duration-200 hover:opacity-90",
                isDark ? "bg-zinc-100 text-zinc-950" : "bg-zinc-900 text-white"
              )}
              onClick={() => setIsUserModalOpen(true)}
              type="button"
            >
              <Plus size={16} strokeWidth={iconStroke} />
              <span className="hidden sm:inline">{t.addUser}</span>
            </button>
          </div>
        }
        icon={Users}
        subtitle={t.usersSub}
        theme={theme}
        title={t.userManagement}
      >
        <div className="space-y-5">
          <div className="grid gap-4 xl:grid-cols-4">
            {[
              {
                icon: Users,
                label: t.summaryUsers,
                value: formatNumber(runtimeSnapshot.counts.users, language)
              },
              {
                icon: Activity,
                label: t.activeUsers,
                value: formatNumber(activeUserCount, language)
              },
              {
                icon: Sparkles,
                label: t.tokens,
                value: formatNumber(runtimeSnapshot.billing.totalTokens, language)
              },
              {
                icon: DollarSign,
                label: t.totalSpend,
                value: formatUsd(runtimeSnapshot.billing.totalSpendUsd, language)
              }
            ].map((item) => {
              const Icon = item.icon;
              return (
                <div
                  className={cx(
                    "rounded-[28px] border p-5 shadow-soft",
                    isDark
                      ? "border-white/10 bg-[#0c0f15]"
                      : "border-zinc-200 bg-zinc-50"
                  )}
                  key={item.label}
                >
                  <div className="flex items-center justify-between gap-3">
                    <span
                      className={cx(
                        "flex h-11 w-11 items-center justify-center rounded-2xl",
                        isDark
                          ? "bg-white/[0.06] text-zinc-200"
                          : "bg-white text-zinc-600"
                      )}
                    >
                      <Icon size={18} strokeWidth={iconStroke} />
                    </span>
                    <p
                      className={cx(
                        "text-[11px] uppercase tracking-[0.18em]",
                        isDark ? "text-zinc-500" : "text-zinc-400"
                      )}
                    >
                      {item.label}
                    </p>
                  </div>
                  <p
                    className={cx(
                      "mt-5 text-2xl font-semibold tracking-tight",
                      isDark ? "text-zinc-50" : "text-zinc-900"
                    )}
                  >
                    {item.value}
                  </p>
                </div>
              );
            })}
          </div>

          {usersWithUsage.length === 0 ? (
            <EmptyState icon={Users} theme={theme} title={t.noUsers} />
          ) : (
            <div className="space-y-4">
              {usersWithUsage.map(({ user, totalTokens }) => (
                <article
                  className={cx(
                    "rounded-[30px] border p-5 shadow-soft transition-all duration-200",
                    selectedUserId === user.id
                      ? isDark
                        ? "border-sky-400/30 bg-white/[0.05]"
                        : "border-sky-200 bg-white"
                      : isDark
                        ? "border-white/10 bg-white/[0.03] hover:bg-white/[0.04]"
                        : "border-zinc-200 bg-zinc-50 hover:bg-white"
                  )}
                  key={user.id}
                >
                  <div className="flex flex-col gap-5 xl:flex-row xl:items-center xl:justify-between">
                    <div className="flex min-w-0 flex-1 items-start gap-4">
                      <span
                        className={cx(
                          "flex h-14 w-14 shrink-0 items-center justify-center rounded-full text-sm font-semibold shadow-soft",
                          isDark
                            ? "bg-white/[0.08] text-zinc-100"
                            : "bg-white text-zinc-700"
                        )}
                      >
                        {initialsForName(user.name)}
                      </span>

                      <div className="min-w-0 flex-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <p
                            className={cx(
                              "text-base font-semibold tracking-tight",
                              isDark ? "text-zinc-50" : "text-zinc-900"
                            )}
                          >
                            {user.name}
                          </p>
                          <span
                            className={cx(
                              "rounded-full px-3 py-1.5 text-xs font-medium",
                              user.role === "admin"
                                ? isDark
                                  ? "bg-sky-500/14 text-sky-200"
                                  : "bg-sky-100 text-sky-700"
                                : isDark
                                  ? "bg-white/[0.06] text-zinc-300"
                                  : "bg-zinc-200 text-zinc-600"
                            )}
                          >
                            {user.role === "admin" ? t.admin : t.viewer}
                          </span>
                        </div>

                        <p
                          className={cx(
                            "mt-1 truncate text-sm",
                            isDark ? "text-zinc-500" : "text-zinc-500"
                          )}
                        >
                          {user.email}
                        </p>

                        <div className="mt-3 flex flex-wrap gap-2">
                          {[
                            {
                              icon: KeyRound,
                              value: user.tokenPreview
                            },
                            {
                              icon: Bot,
                              value: user.defaultModel ?? t.useRequestModel
                            },
                            {
                              icon: Sparkles,
                              value: user.reasoningEffort ?? t.useRequestReasoning
                            }
                          ].map((item) => {
                            const Icon = item.icon;
                            return (
                              <span
                                className={cx(
                                  "inline-flex max-w-full items-center gap-2 rounded-full px-3 py-1.5 text-xs",
                                  isDark
                                    ? "bg-white/[0.06] text-zinc-300"
                                    : "bg-white text-zinc-600"
                                )}
                                key={item.value}
                              >
                                <Icon className="shrink-0" size={13} strokeWidth={iconStroke} />
                                <span className="truncate">{item.value}</span>
                              </span>
                            );
                          })}
                        </div>

                        <p
                          className={cx(
                            "mt-4 text-xs",
                            isDark ? "text-zinc-500" : "text-zinc-500"
                          )}
                        >
                          {t.lastUsed} · {relativeTime(user.lastUsedAt, language)}
                        </p>
                      </div>
                    </div>

                    <div className="grid gap-3 sm:grid-cols-3 xl:min-w-[360px] xl:max-w-[420px]">
                      {[
                        {
                          icon: Sparkles,
                          label: t.tokens,
                          value: formatNumber(totalTokens, language)
                        },
                        {
                          icon: DollarSign,
                          label: t.estimatedSpend,
                          value: formatUsd(user.estimatedSpendUsd, language)
                        },
                        {
                          icon: Activity,
                          label: t.requestCount,
                          value: formatNumber(user.requestCount, language)
                        }
                      ].map((item) => {
                        const Icon = item.icon;
                        return (
                          <div
                            className={cx(
                              "rounded-[22px] border px-4 py-3",
                              isDark
                                ? "border-white/10 bg-[#0d1016]"
                                : "border-zinc-200 bg-white"
                            )}
                            key={item.label}
                          >
                            <p
                              className={cx(
                                "flex items-center gap-2 text-[11px] uppercase tracking-[0.15em]",
                                isDark ? "text-zinc-500" : "text-zinc-400"
                              )}
                            >
                              <Icon size={14} strokeWidth={iconStroke} />
                              {item.label}
                            </p>
                            <p
                              className={cx(
                                "mt-2 text-sm font-medium",
                                isDark ? "text-zinc-100" : "text-zinc-900"
                              )}
                            >
                              {item.value}
                            </p>
                          </div>
                        );
                      })}
                    </div>

                    <button
                      className={cx(
                        "inline-flex items-center justify-center gap-2 rounded-[22px] px-4 py-3 text-sm font-medium transition-all duration-200",
                        isDark
                          ? "bg-zinc-100 text-zinc-950 hover:opacity-90"
                          : "bg-zinc-900 text-white hover:opacity-90"
                      )}
                      onClick={() => setSelectedUserId(user.id)}
                      type="button"
                    >
                      <Settings size={16} strokeWidth={iconStroke} />
                      {t.userConfig}
                    </button>
                  </div>
                </article>
              ))}
            </div>
          )}
        </div>
      </SectionCard>
    </div>
  );

  const renderAlerts = () => (
    <SectionCard
      icon={Bell}
      subtitle={t.alertsSub}
      theme={theme}
      title={t.alerts}
    >
      {runtimeSnapshot.cfIncidents.length === 0 ? (
        <EmptyState icon={BellOff} theme={theme} title={t.noAlerts} />
      ) : (
        <div className="space-y-4">
          {runtimeSnapshot.cfIncidents.map((incident) => (
            <div className="flex gap-4" key={incident.id}>
              <div className="flex flex-col items-center">
                <span
                  className={cx(
                    "flex h-10 w-10 items-center justify-center rounded-2xl shadow-soft",
                    isDark
                      ? "bg-amber-500/14 text-amber-200"
                      : "bg-amber-50 text-amber-600"
                  )}
                >
                  <Bell size={16} strokeWidth={iconStroke} />
                </span>
                <span className={cx("mt-2 h-full w-px", isDark ? "bg-white/10" : "bg-zinc-200")} />
              </div>
              <div
                className={cx(
                  "flex-1 rounded-[24px] border p-4 shadow-soft",
                  isDark
                    ? "border-white/10 bg-white/[0.03]"
                    : "border-zinc-200 bg-zinc-50"
                )}
              >
                <p className={cx("text-sm font-medium", isDark ? "text-zinc-50" : "text-zinc-900")}>
                  {t.lowQuotaDetected(shortId(incident.accountId))}
                </p>
                <p className={cx("mt-2 text-xs leading-5", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {t.alertDescription(
                    incident.accountLabel,
                    incident.severity,
                    incident.cooldownLevel
                  )}
                </p>
                <p className={cx("mt-3 text-xs", isDark ? "text-zinc-600" : "text-zinc-400")}>
                  {relativeTime(incident.happenedAt, language)}
                </p>
              </div>
            </div>
          ))}
        </div>
      )}
    </SectionCard>
  );

  const renderLogs = () => (
    <SectionCard
      icon={FileText}
      subtitle={t.requestLogsSub}
      theme={theme}
      title={t.requestLogs}
    >
      {runtimeSnapshot.requestLogs.length === 0 ? (
        <EmptyState icon={FileText} theme={theme} title={t.noLogs} />
      ) : (
        <div
          className={cx(
            "overflow-hidden rounded-[28px] p-4 shadow-soft",
            isDark ? "bg-[#0b0d12]" : "bg-zinc-950"
          )}
        >
          <div className="mb-3 grid grid-cols-[110px_64px_64px_140px_1fr_110px] gap-3 px-3 text-[11px] uppercase tracking-[0.18em] text-zinc-500">
            <span>Time</span>
            <span>{t.method}</span>
            <span>Status</span>
            <span>{t.user}</span>
            <span>{t.endpoint}</span>
            <span>{t.cost}</span>
          </div>
          <div className="space-y-2 font-mono text-xs">
            {runtimeSnapshot.requestLogs.slice(0, 60).map((log) => (
              <div
                className="grid grid-cols-[110px_64px_64px_140px_1fr_110px] items-center gap-3 rounded-2xl px-3 py-2 text-zinc-300 transition-all duration-200 hover:bg-white/5"
                key={log.id}
              >
                <span className="truncate text-zinc-500">
                  {new Date(log.createdAt).toLocaleTimeString(
                    language === "zh" ? "zh-CN" : "en-US",
                    {
                      hour: "2-digit",
                      minute: "2-digit",
                      second: "2-digit",
                      hour12: false
                    }
                  )}
                </span>
                <span className="text-sky-400">{log.method}</span>
                <span
                  className={cx(
                    log.statusCode >= 400 ? "text-rose-400" : "text-emerald-400"
                  )}
                >
                  {log.statusCode}
                </span>
                <span className="truncate text-zinc-400">{log.userName}</span>
                <span className="truncate">
                  {log.endpoint} · {log.effectiveModel}
                </span>
                <span className="truncate text-right text-zinc-100">
                  {typeof log.estimatedCostUsd === "number"
                    ? formatUsd(log.estimatedCostUsd, language)
                    : "--"}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </SectionCard>
  );

  const renderConfig = () => (
    <SectionCard
      icon={Settings}
      subtitle={t.configSub}
      theme={theme}
      title={t.systemConfig}
    >
      <div className="grid gap-4 xl:grid-cols-2">
        <div
          className={cx(
            "space-y-4 rounded-[28px] p-4",
            isDark ? "bg-[#0c0f15]" : "bg-zinc-50"
          )}
        >
          <ToggleRow
            checked={protectionMode}
            description={t.protectionModeDesc}
            label={t.protectionMode}
            onChange={() => setProtectionMode((value) => !value)}
            theme={theme}
          />
          <ToggleRow
            checked={autoRefill}
            description={t.autoRefillDesc}
            label={t.autoRefill}
            onChange={() => setAutoRefill((value) => !value)}
            theme={theme}
          />
          <ToggleRow
            checked={theme === "dark"}
            description={theme === "dark" ? t.darkTheme : t.lightTheme}
            label={t.theme}
            onChange={() =>
              setTheme((current) => (current === "dark" ? "light" : "dark"))
            }
            theme={theme}
          />
        </div>

        <div
          className={cx(
            "space-y-3 rounded-[28px] p-4",
            isDark ? "bg-[#0c0f15]" : "bg-zinc-50"
          )}
        >
          <div
            className={cx(
              "rounded-[24px] border p-4 shadow-soft",
              isDark
                ? "border-white/10 bg-white/[0.03]"
                : "border-zinc-200 bg-white"
            )}
          >
            <div className="flex items-center gap-3">
              <span
                className={cx(
                  "flex h-10 w-10 items-center justify-center rounded-2xl",
                  isDark ? "bg-white/[0.05] text-zinc-300" : "bg-zinc-100 text-zinc-600"
                )}
              >
                <Database size={18} strokeWidth={iconStroke} />
              </span>
              <div>
                <p className={cx("text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                  {t.storage}
                </p>
                <p className={cx("mt-1 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {health.storageMode}
                </p>
              </div>
            </div>
          </div>

          <div
            className={cx(
              "rounded-[24px] border p-4 shadow-soft",
              isDark
                ? "border-white/10 bg-white/[0.03]"
                : "border-zinc-200 bg-white"
            )}
          >
            <div className="flex items-center gap-3">
              <span
                className={cx(
                  "flex h-10 w-10 items-center justify-center rounded-2xl",
                  isDark ? "bg-white/[0.05] text-zinc-300" : "bg-zinc-100 text-zinc-600"
                )}
              >
                <Sparkles size={18} strokeWidth={iconStroke} />
              </span>
              <div>
                <p className={cx("text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                  {t.services}
                </p>
                <p className={cx("mt-1 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  Redis {health.redisConnected ? t.online : t.offline} · Postgres{" "}
                  {health.postgresConnected ? t.online : t.offline}
                </p>
              </div>
            </div>
          </div>

          <div
            className={cx(
              "rounded-[24px] border p-4 shadow-soft",
              isDark
                ? "border-white/10 bg-white/[0.03]"
                : "border-zinc-200 bg-white"
            )}
          >
            <div className="flex items-center gap-3">
              <span
                className={cx(
                  "flex h-10 w-10 items-center justify-center rounded-2xl",
                  isDark ? "bg-white/[0.05] text-zinc-300" : "bg-zinc-100 text-zinc-600"
                )}
              >
                {theme === "dark" ? (
                  <MoonStar size={18} strokeWidth={iconStroke} />
                ) : (
                  <SunMedium size={18} strokeWidth={iconStroke} />
                )}
              </span>
              <div>
                <p className={cx("text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                  {t.theme}
                </p>
                <p className={cx("mt-1 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {theme === "dark" ? t.darkTheme : t.lightTheme}
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>
    </SectionCard>
  );

  const renderActiveView = () => {
    switch (activeView) {
      case "accounts":
        return renderAccounts();
      case "users":
        return renderUsers();
      case "alerts":
        return renderAlerts();
      case "logs":
        return renderLogs();
      case "config":
        return renderConfig();
      default:
        return renderOverview();
    }
  };

  return (
    <div
      className={cx(
        "min-h-screen transition-colors duration-300",
        isDark ? "bg-transparent text-zinc-100" : "bg-transparent text-zinc-900"
      )}
    >
      <aside
        className={cx(
          "fixed inset-y-4 left-4 z-40 hidden rounded-[34px] border p-2.5 shadow-panel backdrop-blur-xl transition-all duration-200 md:flex md:flex-col",
          isDark
            ? "border-white/10 bg-[#10131a]/88"
            : "border-white/70 bg-white/85",
          effectiveSidebarExpanded ? "w-[240px]" : "w-[72px]"
        )}
        onMouseEnter={() => setSidebarHovered(true)}
        onMouseLeave={() => setSidebarHovered(false)}
      >
        <div
          className={cx(
            "mb-6 flex items-center overflow-hidden",
            effectiveSidebarExpanded ? "gap-3" : "justify-center"
          )}
        >
          <div className="flex items-center gap-3 overflow-hidden">
            <span
              className={cx(
                "flex h-11 w-11 shrink-0 items-center justify-center rounded-[18px]",
                isDark ? "bg-zinc-100 text-zinc-950" : "bg-zinc-900 text-white"
              )}
            >
              <Bot size={18} strokeWidth={iconStroke} />
            </span>
            <div
              className={cx(
                "transition-all duration-200",
                effectiveSidebarExpanded ? "opacity-100" : "max-w-0 opacity-0"
              )}
            >
              <p className={cx("text-sm font-semibold", isDark ? "text-zinc-50" : "text-zinc-900")}>
                {t.brandTitle}
              </p>
              <p className={cx("text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                {t.brandSubtitle}
              </p>
            </div>
          </div>
        </div>

        <nav
          className={cx(
            "flex-1 space-y-2",
            effectiveSidebarExpanded ? "" : "flex flex-col items-center"
          )}
        >
          {navItems.map((item) => (
            <SidebarItem
              active={activeView === item.id}
              expanded={effectiveSidebarExpanded}
              icon={item.icon}
              key={item.id}
              label={item.label}
              onClick={() => setActiveView(item.id)}
              theme={theme}
            />
          ))}
        </nav>

        <div className="pt-3">
          {effectiveSidebarExpanded ? (
            <div
              className={cx(
                "rounded-[30px] border p-3",
                isDark
                  ? "border-white/10 bg-white/[0.04]"
                  : "border-zinc-200 bg-zinc-100/80"
              )}
            >
              <div className="flex items-center gap-3">
                <span
                  className={cx(
                    "relative flex h-11 w-11 shrink-0 items-center justify-center rounded-[18px] shadow-soft",
                    isDark ? "bg-white/[0.06] text-zinc-300" : "bg-white text-zinc-500"
                  )}
                >
                  <Sparkles size={16} strokeWidth={iconStroke} />
                  <span
                    className={cx(
                      "absolute right-2 top-2 h-2 w-2 rounded-full ring-2",
                      health.status === "ok" ? "bg-emerald-400" : "bg-amber-400",
                      isDark ? "ring-[#141821]" : "ring-white"
                    )}
                  />
                </span>
                <div className="min-w-0">
                  <p className={cx("text-xs font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                    {health.status === "ok" ? t.systemReady : t.checkSystem}
                  </p>
                  <p className={cx("mt-1 text-[11px]", isDark ? "text-zinc-500" : "text-zinc-500")}>
                    {t.accountsInPool(runtimeSnapshot.counts.accounts)}
                  </p>
                </div>
              </div>

              <div className={cx("my-3 h-px", isDark ? "bg-white/10" : "bg-zinc-200")} />

              <div className="flex justify-end">
                <button
                  className={cx(
                    "flex h-11 items-center justify-center gap-2 rounded-full px-4 transition-all duration-200",
                    isDark
                      ? "bg-white/[0.08] text-zinc-200 hover:bg-white/[0.12]"
                      : "bg-white text-zinc-600 hover:bg-zinc-50"
                  )}
                  onClick={() => setSidebarExpanded((value) => !value)}
                  type="button"
                >
                  <ChevronLeft size={16} strokeWidth={iconStroke} />
                  <span className="text-sm font-medium">{t.collapse}</span>
                </button>
              </div>
            </div>
          ) : (
            <div
              className={cx(
                "mx-auto flex w-12 flex-col items-center gap-2 rounded-[24px] border p-1.5",
                isDark
                  ? "border-white/10 bg-white/[0.04]"
                  : "border-zinc-200 bg-zinc-100/80"
              )}
            >
              <span
                className={cx(
                  "relative flex h-9 w-9 items-center justify-center rounded-[16px] shadow-soft",
                  isDark ? "bg-white/[0.06] text-zinc-300" : "bg-white text-zinc-500"
                )}
                title={health.status === "ok" ? t.systemReady : t.checkSystem}
              >
                <Sparkles size={15} strokeWidth={iconStroke} />
                <span
                  className={cx(
                    "absolute right-1.5 top-1.5 h-2 w-2 rounded-full ring-2",
                    health.status === "ok" ? "bg-emerald-400" : "bg-amber-400",
                    isDark ? "ring-[#141821]" : "ring-white"
                  )}
                />
              </span>

              <div className={cx("h-px w-6", isDark ? "bg-white/10" : "bg-zinc-200")} />

              <button
                className={cx(
                  "flex h-9 w-9 items-center justify-center rounded-[16px] transition-all duration-200",
                  isDark
                    ? "bg-white/[0.08] text-zinc-200 hover:bg-white/[0.12]"
                    : "bg-white text-zinc-600 hover:bg-zinc-50"
                )}
                onClick={() => setSidebarExpanded((value) => !value)}
                title={t.expand}
                type="button"
              >
                <ChevronRight size={15} strokeWidth={iconStroke} />
              </button>
            </div>
          )}
        </div>
      </aside>

      <aside
        className={cx(
          "fixed inset-x-4 bottom-4 z-40 rounded-[28px] border p-3 shadow-panel backdrop-blur-xl md:hidden",
          isDark
            ? "border-white/10 bg-[#10131a]/92"
            : "border-white/70 bg-white/90"
        )}
      >
        <div className="grid grid-cols-6 gap-2">
          {navItems.map((item) => {
            const Icon = item.icon;
            const active = activeView === item.id;
            return (
              <button
                className={cx(
                  "flex h-11 items-center justify-center rounded-2xl transition-all duration-200",
                  active
                    ? isDark
                      ? "bg-sky-500/14 text-sky-200"
                      : "bg-sky-50 text-sky-600"
                    : isDark
                      ? "bg-white/[0.05] text-zinc-500"
                      : "bg-zinc-100 text-zinc-500"
                )}
                key={item.id}
                onClick={() => setActiveView(item.id)}
                type="button"
              >
                <Icon size={18} strokeWidth={iconStroke} />
              </button>
            );
          })}
        </div>
      </aside>

      <main
        className={cx(
          "min-h-screen p-4 pb-28 transition-all duration-200 md:p-6 md:pb-6",
          effectiveSidebarExpanded ? "md:pl-[264px]" : "md:pl-[104px]"
        )}
      >
        <div className="mx-auto max-w-[1600px] space-y-6">
          <header
            className={cx(
              "rounded-[36px] border p-6 shadow-soft backdrop-blur-xl",
              isDark
                ? "border-white/10 bg-[#10131a]/72"
                : "border-white/70 bg-white/70"
            )}
          >
            <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
              <div>
                <p className={cx("text-xs uppercase tracking-[0.24em]", isDark ? "text-zinc-500" : "text-zinc-400")}>
                  {t.headerKicker}
                </p>
                <h1 className={cx("mt-3 text-xl font-semibold tracking-tight md:text-3xl", isDark ? "text-zinc-50" : "text-zinc-900")}>
                  {t.headerTitle}
                </h1>
                <p className={cx("mt-3 max-w-3xl text-sm leading-6", isDark ? "text-zinc-400" : "text-zinc-500")}>
                  {t.headerDescription}
                </p>
              </div>

              <div className="flex flex-wrap items-center gap-3">
                <div
                  className={cx(
                    "flex items-center gap-1 rounded-2xl p-1 shadow-soft",
                    isDark ? "bg-white/[0.06]" : "bg-white"
                  )}
                >
                  <span className={cx("px-2 text-[11px] font-medium uppercase tracking-[0.18em]", isDark ? "text-zinc-500" : "text-zinc-400")}>
                    {t.language}
                  </span>
                  {(["zh", "en"] as Language[]).map((value) => (
                    <button
                      className={cx(
                        "rounded-xl px-3 py-2 text-xs font-medium transition-all duration-200",
                        language === value
                          ? isDark
                            ? "bg-zinc-100 text-zinc-950"
                            : "bg-zinc-900 text-white"
                          : isDark
                            ? "text-zinc-400 hover:bg-white/[0.06]"
                            : "text-zinc-500 hover:bg-zinc-100"
                      )}
                      key={value}
                      onClick={() => setLanguage(value)}
                      type="button"
                    >
                      {value === "zh" ? "CN" : "EN"}
                    </button>
                  ))}
                </div>

                <button
                  className={cx(
                    "inline-flex items-center gap-2 rounded-2xl px-4 py-3 text-sm font-medium transition-all duration-200",
                    isDark
                      ? "bg-white/[0.06] text-zinc-200 hover:bg-white/[0.1]"
                      : "bg-white text-zinc-600 hover:bg-zinc-100"
                  )}
                  onClick={() =>
                    setTheme((current) =>
                      current === "dark" ? "light" : "dark"
                    )
                  }
                  type="button"
                >
                  {theme === "dark" ? (
                    <MoonStar size={16} strokeWidth={iconStroke} />
                  ) : (
                    <SunMedium size={16} strokeWidth={iconStroke} />
                  )}
                  {theme === "dark" ? t.darkTheme : t.lightTheme}
                </button>

                {[
                  {
                    label: t.summaryCache,
                    value: `${percent(runtimeSnapshot.cacheMetrics.prefixHitRatio)}%`
                  },
                  {
                    label: t.summaryUsers,
                    value: formatNumber(runtimeSnapshot.counts.users, language)
                  },
                  {
                    label: t.summarySpend,
                    value: formatUsd(runtimeSnapshot.billing.totalSpendUsd, language)
                  },
                  {
                    label: t.summaryHealth,
                    value: health.status === "ok" ? t.nominal : t.attention
                  }
                ].map((item) => (
                  <div
                    className={cx(
                      "rounded-2xl px-4 py-3 shadow-soft",
                      isDark ? "bg-white/[0.06]" : "bg-white"
                    )}
                    key={item.label}
                  >
                    <p className={cx("text-[11px] uppercase tracking-[0.2em]", isDark ? "text-zinc-500" : "text-zinc-400")}>
                      {item.label}
                    </p>
                    <p className={cx("mt-1 text-sm font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                      {item.value}
                    </p>
                  </div>
                ))}
              </div>
            </div>
          </header>

          {banner ? (
            <div
              className={cx(
                "rounded-[24px] px-4 py-3 text-sm shadow-soft",
                banner.tone === "ok"
                  ? isDark
                    ? "bg-emerald-500/10 text-emerald-200"
                    : "bg-emerald-50 text-emerald-700"
                  : banner.tone === "error"
                    ? isDark
                      ? "bg-rose-500/10 text-rose-200"
                      : "bg-rose-50 text-rose-700"
                    : isDark
                      ? "bg-white/[0.05] text-zinc-200"
                      : "bg-zinc-100 text-zinc-700"
              )}
            >
              {banner.message}
            </div>
          ) : null}

          {renderActiveView()}
        </div>
      </main>

      <div
        className={cx(
          "fixed inset-0 z-50 transition-all duration-200",
          isDrawerOpen ? "pointer-events-auto" : "pointer-events-none"
        )}
      >
        <div
          className={cx(
            "absolute inset-0 transition-opacity duration-200",
            isDark ? "bg-[#05070b]/72" : "bg-zinc-900/20",
            isDrawerOpen ? "opacity-100" : "opacity-0"
          )}
          onClick={() => setIsDrawerOpen(false)}
        />
        <div
          className={cx(
            "absolute inset-y-4 right-4 w-full max-w-[420px] rounded-[32px] border p-6 shadow-panel backdrop-blur-xl transition-all duration-200",
            isDark
              ? "border-white/10 bg-[#11141b]/95"
              : "border-white/70 bg-white/95",
            isDrawerOpen ? "translate-x-0 opacity-100" : "translate-x-8 opacity-0"
          )}
        >
          <div className="flex items-center justify-between">
            <div>
              <p className={cx("text-xs uppercase tracking-[0.22em]", isDark ? "text-zinc-500" : "text-zinc-400")}>
                {t.drawerKicker}
              </p>
              <h3 className={cx("mt-2 text-xl font-semibold", isDark ? "text-zinc-50" : "text-zinc-900")}>
                {t.drawerTitle}
              </h3>
            </div>
            <button
              className={cx(
                "flex h-10 w-10 items-center justify-center rounded-2xl transition-all duration-200",
                isDark
                  ? "bg-white/[0.06] text-zinc-400 hover:bg-white/[0.1]"
                  : "bg-zinc-100 text-zinc-500 hover:bg-zinc-200"
              )}
              onClick={() => setIsDrawerOpen(false)}
              type="button"
            >
              <ChevronRight size={18} strokeWidth={iconStroke} />
            </button>
          </div>

          <div className="mt-6 grid grid-cols-2 gap-3">
            {[
              { label: t.active, value: activeAccounts.length },
              { label: t.lowQuota, value: lowQuotaAccounts.length },
              { label: t.protected, value: protectedAccounts.length },
              {
                label: t.totalSpend,
                value: formatUsd(runtimeSnapshot.billing.totalSpendUsd, language)
              }
            ].map((item) => (
              <div
                className={cx(
                  "rounded-[24px] p-4 text-center",
                  isDark ? "bg-[#0c0f15]" : "bg-zinc-50"
                )}
                key={item.label}
              >
                <p className={cx("text-[11px] uppercase tracking-[0.18em]", isDark ? "text-zinc-500" : "text-zinc-400")}>
                  {item.label}
                </p>
                <p className={cx("mt-2 text-lg font-semibold", isDark ? "text-zinc-50" : "text-zinc-900")}>
                  {item.value}
                </p>
              </div>
            ))}
          </div>

          <div className="mt-5 grid gap-3">
            <button
              className={cx(
                "flex items-center justify-between rounded-[24px] px-4 py-3 text-sm font-medium transition-all duration-200 hover:opacity-90",
                isDark ? "bg-zinc-100 text-zinc-950" : "bg-zinc-900 text-white"
              )}
              onClick={() => {
                setActiveView("accounts");
                setIsDrawerOpen(false);
              }}
              type="button"
            >
              <span className="flex min-w-0 flex-1 items-center gap-3 text-left">
                <Bot size={17} strokeWidth={iconStroke} />
                <span className="truncate">{t.syncSnapshot}</span>
              </span>
              <ChevronRight className="shrink-0" size={16} strokeWidth={iconStroke} />
            </button>
            <button
              className={cx(
                "flex items-center justify-between rounded-[24px] px-4 py-3 text-sm font-medium transition-all duration-200",
                isDark
                  ? "bg-white/[0.06] text-zinc-200 hover:bg-white/[0.1]"
                  : "bg-zinc-100 text-zinc-700 hover:bg-zinc-200"
              )}
              onClick={() => {
                setActiveView("users");
                setIsDrawerOpen(false);
              }}
              type="button"
            >
              <span className="flex min-w-0 flex-1 items-center gap-3 text-left">
                <Users size={17} strokeWidth={iconStroke} />
                <span className="truncate">{t.reviewProtected}</span>
              </span>
              <ChevronRight className="shrink-0" size={16} strokeWidth={iconStroke} />
            </button>
          </div>

          <div className="mt-6">
            <div className={cx("mb-3 flex items-center gap-2 text-xs uppercase tracking-[0.2em]", isDark ? "text-zinc-500" : "text-zinc-400")}>
              <Clock3 size={14} strokeWidth={iconStroke} />
              {t.priorityAccounts}
            </div>
            <div className="space-y-3">
              {priorityAccounts.length === 0 ? (
                <EmptyState
                  icon={BellOff}
                  theme={theme}
                  title={t.noPriorityAccounts}
                />
              ) : (
                priorityAccounts.map((account) => {
                  const kind = accountKind(account);
                  return (
                    <div
                      className={cx(
                        "flex items-center justify-between rounded-[24px] border p-4 shadow-soft",
                        isDark
                          ? "border-white/10 bg-white/[0.03]"
                          : "border-zinc-200 bg-zinc-50"
                      )}
                      key={account.id}
                    >
                      <div className="flex items-center gap-3">
                        <span
                          className={cx(
                            "h-2.5 w-2.5 rounded-full",
                            kind === "active"
                              ? "bg-emerald-500"
                              : kind === "protected"
                                ? "bg-amber-500"
                                : "bg-rose-500"
                          )}
                        />
                        <div>
                          <p className={cx("text-sm font-medium", isDark ? "text-zinc-50" : "text-zinc-900")}>
                            {account.label || shortId(account.id)}
                          </p>
                          <p className={cx("mt-1 text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                            {shortId(account.id)} · {account.models[0] ?? "gpt-5.4"} ·{" "}
                            {percent(account.quotaHeadroom)}%
                          </p>
                        </div>
                      </div>
                      <span
                        className={cx(
                          "rounded-full px-3 py-1.5 text-xs font-medium",
                          kind === "active"
                            ? isDark
                              ? "bg-emerald-500/14 text-emerald-200"
                              : "bg-emerald-100 text-emerald-700"
                            : kind === "protected"
                              ? isDark
                                ? "bg-amber-500/14 text-amber-200"
                                : "bg-amber-100 text-amber-700"
                              : isDark
                                ? "bg-rose-500/14 text-rose-200"
                                : "bg-rose-100 text-rose-700"
                        )}
                      >
                        {kind === "active"
                          ? t.active
                          : kind === "protected"
                            ? t.protected
                            : t.lowQuota}
                      </span>
                    </div>
                  );
                })
              )}
            </div>
          </div>
        </div>
      </div>

      <AccountAddDrawer
        accountCount={runtimeSnapshot.counts.accounts}
        callbackUrl={callbackUrl}
        language={language}
        onClose={() => setIsAccountDrawerOpen(false)}
        open={isAccountDrawerOpen}
        tenantCount={runtimeSnapshot.counts.tenants}
        theme={theme}
      />

      <div
        className={cx(
          "fixed inset-0 z-[65] transition-all duration-200",
          selectedUserEntry ? "pointer-events-auto" : "pointer-events-none"
        )}
      >
        <div
          className={cx(
            "absolute inset-0 transition-opacity duration-200",
            isDark ? "bg-[#05070b]/72" : "bg-zinc-900/20",
            selectedUserEntry ? "opacity-100" : "opacity-0"
          )}
          onClick={() => setSelectedUserId(null)}
        />

        <div
          className={cx(
            "absolute inset-y-3 right-3 flex w-full max-w-[680px] flex-col overflow-hidden rounded-[34px] border shadow-panel backdrop-blur-xl transition-all duration-200",
            isDark
              ? "border-white/10 bg-[#11141b]/96"
              : "border-white/70 bg-white/96",
            selectedUserEntry ? "translate-x-0 opacity-100" : "translate-x-8 opacity-0"
          )}
        >
          {selectedUserEntry ? (
            <>
              <div
                className={cx(
                  "flex items-start justify-between gap-4 border-b px-6 py-5",
                  isDark ? "border-white/10" : "border-zinc-200/70"
                )}
              >
                <div className="min-w-0">
                  <p
                    className={cx(
                      "text-[11px] uppercase tracking-[0.22em]",
                      isDark ? "text-zinc-500" : "text-zinc-400"
                    )}
                  >
                    {t.userConfig}
                  </p>
                  <h3
                    className={cx(
                      "mt-2 text-xl font-semibold tracking-tight",
                      isDark ? "text-zinc-50" : "text-zinc-900"
                    )}
                  >
                    {selectedUserEntry.user.name}
                  </h3>
                  <p
                    className={cx(
                      "mt-2 max-w-xl text-xs leading-6",
                      isDark ? "text-zinc-500" : "text-zinc-500"
                    )}
                  >
                    {t.userConfigHint}
                  </p>
                </div>

                <button
                  className={cx(
                    "flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl transition-all duration-200",
                    isDark
                      ? "bg-white/[0.06] text-zinc-400 hover:bg-white/[0.1]"
                      : "bg-zinc-100 text-zinc-500 hover:bg-zinc-200"
                  )}
                  onClick={() => setSelectedUserId(null)}
                  type="button"
                >
                  <ChevronRight size={18} strokeWidth={iconStroke} />
                </button>
              </div>

              <div className="flex-1 overflow-y-auto p-6 pb-8">
                <UserPolicyCard
                  availableModels={availableModels}
                  compact
                  gatewayBaseUrl={gatewayBaseUrl}
                  language={language}
                  latestIssuedToken={
                    latestCreatedUser?.user.id === selectedUserEntry.user.id
                      ? latestCreatedUser.token
                      : null
                  }
                  onSave={handleSaveUser}
                  recentInstances={selectedUserEntry.recentInstances}
                  saving={savingUserId === selectedUserEntry.user.id}
                  theme={theme}
                  tokenUsage={selectedUserEntry.totalTokens}
                  user={selectedUserEntry.user}
                />
              </div>
            </>
          ) : null}
        </div>
      </div>

      <UserCreateModal
        availableModels={availableModels}
        existingEmails={runtimeSnapshot.users.map((user) => user.email)}
        gatewayBaseUrl={gatewayBaseUrl}
        language={language}
        onClose={() => setIsUserModalOpen(false)}
        onCreated={handleUserCreated}
        open={isUserModalOpen}
        theme={theme}
      />
    </div>
  );
}

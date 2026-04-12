"use client";

import { useEffect, useMemo, useState } from "react";
import type {
  AccountCleanupResult,
  DashboardLiveSnapshot,
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
  CircleX,
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
  Trash2,
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

type AccountFilter = "all" | "available" | "inUse" | "disabled";
type Language = "zh" | "en";
type ThemeMode = "dark" | "light";
type BannerTone = "ok" | "error" | "neutral";
type LiveRefreshInterval = 5000 | 10000 | 30000 | 0;

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

type AccountRecord = DashboardSnapshot["accounts"][number];
type AccountAlertRecord = DashboardSnapshot["accountAlerts"][number];

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
const managedQuotaRefreshIntervalMs = 60_000;

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

function truncateText(value: string, limit = 120) {
  return value.length > limit ? `${value.slice(0, limit - 3)}...` : value;
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

function formatMToken(value: number, language: Language) {
  const mtokens = value / 1_000_000;
  const formatter = new Intl.NumberFormat(language === "zh" ? "zh-CN" : "en-US", {
    minimumFractionDigits: mtokens >= 10 ? 1 : 2,
    maximumFractionDigits: mtokens >= 10 ? 1 : 2
  });
  return `${formatter.format(mtokens)} MToken`;
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

function ringSegmentStyle(
  value: number,
  radius: number,
  startFraction: number,
  spanFraction: number
) {
  const circumference = 2 * Math.PI * radius;
  const segmentLength = circumference * clamp01(spanFraction) * clamp01(value);
  return {
    strokeDasharray: `${segmentLength} ${Math.max(circumference - segmentLength, 0)}`,
    strokeDashoffset: -circumference * clamp01(startFraction)
  };
}

function requestHitRatio(cacheMetrics: DashboardSnapshot["cacheMetrics"]) {
  return clamp01(cacheMetrics.requestHitRatio ?? cacheMetrics.prefixHitRatio);
}

function tokenHitRatio(cacheMetrics: DashboardSnapshot["cacheMetrics"]) {
  if (Number.isFinite(cacheMetrics.tokenHitRatio)) {
    return clamp01(cacheMetrics.tokenHitRatio);
  }
  if (cacheMetrics.replayTokens <= 0) {
    return 0;
  }
  return clamp01(cacheMetrics.cachedTokens / cacheMetrics.replayTokens);
}

function accountKind(account: AccountRecord) {
  const quotaFloor = Math.min(
    account.quotaHeadroom,
    account.quotaHeadroom5h,
    account.quotaHeadroom7d
  );

  if (
    account.availabilityState === "quota_exhausted" ||
    account.availabilityState === "cooldown" ||
    account.nearQuotaGuardEnabled
  ) {
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

function accountStatusTone(account: AccountRecord) {
  if (account.availabilityState === "quota_exhausted") {
    return {
      dot: "bg-rose-500",
      chip: "bg-rose-500/12 text-rose-300",
      chipLight: "bg-rose-50 text-rose-600"
    };
  }
  if (account.availabilityState === "cooldown") {
    return {
      dot: "bg-amber-500",
      chip: "bg-amber-500/12 text-amber-300",
      chipLight: "bg-amber-50 text-amber-600"
    };
  }
  if (account.status === "banned") {
    return {
      dot: "bg-rose-500",
      chip: "bg-rose-500/12 text-rose-300",
      chipLight: "bg-rose-50 text-rose-600"
    };
  }
  if (status === "unavailable") {
    return {
      dot: "bg-amber-500",
      chip: "bg-amber-500/12 text-amber-300",
      chipLight: "bg-amber-50 text-amber-600"
    };
  }
  return {
    dot: "bg-emerald-500",
    chip: "bg-emerald-500/12 text-emerald-300",
    chipLight: "bg-emerald-50 text-emerald-600"
  };
}

function accountRateLimitWindow(
  account: AccountRecord,
  target: "5h" | "7d"
) {
  const windows = [account.rateLimits?.primary, account.rateLimits?.secondary].reduce<
    Array<NonNullable<NonNullable<AccountRecord["rateLimits"]>["primary"]>>
  >((items, window) => {
    if (window) {
      items.push(window);
    }
    return items;
  }, []);
  return windows.find((window) => {
    const minutes = window.windowDurationMins;
    if (minutes === null || minutes === undefined) {
      return false;
    }
    return target === "5h" ? minutes <= 300 : minutes >= 10_080;
  });
}

function accountQuotaHeadroomLabel(
  account: AccountRecord,
  target: "5h" | "7d"
) {
  return target === "5h"
    ? percent(account.quotaHeadroom5h)
    : percent(account.quotaHeadroom7d);
}

function accountPrimaryIdentifier(account: AccountRecord) {
  return account.chatgptEmail || account.label || shortId(account.id);
}

function accountSecondaryIdentifier(account: AccountRecord) {
  return account.chatgptEmail ? account.label || shortId(account.id) : shortId(account.id);
}

function accountOperationalState(
  account: AccountRecord,
  leasedAccountIds: Set<string>
) {
  if (account.availabilityState !== "routable") {
    return "disabled" as const;
  }
  if (account.inflight > 0 || leasedAccountIds.has(account.id)) {
    return "inUse" as const;
  }
  return "available" as const;
}

function effectiveQuotaHeadroom(account: AccountRecord) {
  return Math.min(
    account.quotaHeadroom,
    account.quotaHeadroom5h,
    account.quotaHeadroom7d
  );
}

function deriveAccountAlertsFromAccounts(accounts: AccountRecord[]): AccountAlertRecord[] {
  const now = new Date().toISOString();
  return accounts
    .reduce<AccountAlertRecord[]>((alerts, account) => {
      const quotaFloor = effectiveQuotaHeadroom(account);

      if (account.availabilityState === "quota_exhausted") {
        alerts.push({
          id: `${account.id}:quota_exhausted`,
          accountId: account.id,
          accountLabel: account.label,
          kind: "quota_exhausted",
          severity: "critical",
          happenedAt: account.availabilityResetAt ?? now,
          quotaHeadroom: quotaFloor,
          cooldownLevel: account.cooldownLevel,
          status: account.status,
          reason: account.availabilityReason ?? "quota_exhausted"
        });
        return alerts;
      }

      if (account.status === "banned") {
        alerts.push({
          id: `${account.id}:disabled`,
          accountId: account.id,
          accountLabel: account.label,
          kind: "disabled",
          severity: "critical",
          happenedAt: account.managedStateRefreshedAt ?? now,
          quotaHeadroom: quotaFloor,
          cooldownLevel: account.cooldownLevel,
          status: account.status,
          reason: account.statusReason ?? "account_disabled"
        });
        return alerts;
      }

      if (account.availabilityState === "unavailable") {
        alerts.push({
          id: `${account.id}:unavailable`,
          accountId: account.id,
          accountLabel: account.label,
          kind: "unavailable",
          severity: "warning",
          happenedAt: account.managedStateRefreshedAt ?? now,
          quotaHeadroom: quotaFloor,
          cooldownLevel: account.cooldownLevel,
          status: account.status,
          reason: account.availabilityReason ?? account.statusReason ?? "refresh_failed"
        });
        return alerts;
      }

      if (account.availabilityState === "cooldown" || account.nearQuotaGuardEnabled) {
        alerts.push({
          id: `${account.id}:protected`,
          accountId: account.id,
          accountLabel: account.label,
          kind: "protected",
          severity: "warning",
          happenedAt:
            account.cooldownUntil ?? account.managedStateRefreshedAt ?? now,
          quotaHeadroom: quotaFloor,
          cooldownLevel: account.cooldownLevel,
          status: account.status,
          reason: account.availabilityReason ?? account.statusReason ?? "cooldown_guard"
        });
        return alerts;
      }

      if (quotaFloor <= 0.25) {
        alerts.push({
          id: `${account.id}:low_quota`,
          accountId: account.id,
          accountLabel: account.label,
          kind: "low_quota",
          severity: quotaFloor <= 0.1 ? "critical" : "warning",
          happenedAt: account.managedStateRefreshedAt ?? now,
          quotaHeadroom: quotaFloor,
          cooldownLevel: account.cooldownLevel,
          status: account.status,
          reason: "quota_floor"
        });
      }

      return alerts;
    }, [])
    .sort((left, right) => {
      const leftPriority = left.severity === "critical" ? 0 : 1;
      const rightPriority = right.severity === "critical" ? 0 : 1;
      if (leftPriority !== rightPriority) {
        return leftPriority - rightPriority;
      }
      return (
        new Date(right.happenedAt).getTime() - new Date(left.happenedAt).getTime()
      );
    });
}

function nextSnapshotWithAccounts(
  current: DashboardSnapshot,
  accounts: DashboardSnapshot["accounts"]
) {
  const activeAccountIds = new Set(accounts.map((account) => account.id));
  const leases = current.leases.filter((lease) => activeAccountIds.has(lease.accountId));
  return {
    ...current,
    accounts,
    leases,
    accountAlerts: deriveAccountAlertsFromAccounts(accounts),
    modelCatalog: Array.from(
      new Set(accounts.flatMap((account) => account.models))
    ).sort(),
    counts: {
      ...current.counts,
      accounts: accounts.length,
      activeLeases: leases.length,
      warpAccounts: accounts.filter((account) => account.routeMode === "warp").length
    }
  };
}

function readApiErrorMessage(payload: unknown, fallback: string) {
  if (
    payload &&
    typeof payload === "object" &&
    !Array.isArray(payload) &&
    "error" in payload
  ) {
    const error = (payload as { error?: { message?: string } }).error;
    if (error?.message) {
      return error.message;
    }
  }
  return fallback;
}

function isAccountRecordPayload(payload: unknown): payload is AccountRecord {
  return (
    payload !== null &&
    typeof payload === "object" &&
    !Array.isArray(payload) &&
    "id" in payload &&
    "models" in payload
  );
}

function isAccountCleanupResultPayload(
  payload: unknown
): payload is AccountCleanupResult {
  return (
    payload !== null &&
    typeof payload === "object" &&
    !Array.isArray(payload) &&
    "deletedAccountIds" in payload &&
    Array.isArray((payload as AccountCleanupResult).deletedAccountIds)
  );
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
        "group flex h-10 items-center rounded-[16px] text-[13px] font-medium transition-all duration-200",
        expanded ? "w-full gap-2.5 px-2.5" : "mx-auto w-10 justify-center px-0",
        active
          ? isDark
            ? "bg-white/[0.08] text-zinc-50 shadow-soft"
            : "bg-white text-zinc-950 shadow-soft"
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
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-[14px] transition-all duration-200",
          active
            ? isDark
              ? "bg-white/[0.12] text-zinc-100"
              : "bg-zinc-900 text-white"
            : isDark
              ? "bg-white/[0.05] text-zinc-400 group-hover:bg-white/[0.08]"
              : "bg-zinc-100 text-zinc-500 group-hover:bg-zinc-200/80"
        )}
      >
        <Icon size={16} strokeWidth={iconStroke} />
      </span>
      <span
        className={cx(
          "overflow-hidden whitespace-nowrap text-left transition-all duration-200",
          expanded ? "max-w-[144px] opacity-100" : "max-w-0 opacity-0"
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
        "apple-panel rounded-[30px] p-4 shadow-panel md:p-5"
      )}
    >
      <header className="mb-5 flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div className="flex items-center gap-2.5">
          <span
            className={cx(
              "apple-subtle-panel flex h-10 w-10 items-center justify-center rounded-[16px]",
              isDark ? "text-zinc-300" : "text-zinc-600"
            )}
          >
            <Icon size={16} strokeWidth={iconStroke} />
          </span>
          <div>
            <h2
              className={cx(
                "text-[18px] font-semibold tracking-[-0.03em]",
                isDark ? "text-zinc-50" : "text-zinc-900"
              )}
            >
              {title}
            </h2>
            {subtitle ? (
              <p
                className={cx(
                  "mt-0.5 text-[11px] leading-[18px]",
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
        "apple-subtle-panel flex min-h-[240px] flex-col items-center justify-center gap-3 rounded-[24px] border border-dashed text-center",
        isDark
          ? "border-white/10"
          : "border-zinc-200"
      )}
    >
      <span
        className={cx(
          "apple-subtle-panel flex h-16 w-16 items-center justify-center rounded-full shadow-soft",
          isDark ? "text-zinc-600" : "text-zinc-300"
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
        brandTitle: "Origin管理端",
        brandSubtitle: "",
        navOverview: "概览",
        navAccounts: "账号",
        navUsers: "用户",
        navAlerts: "告警",
        navLogs: "日志",
        navConfig: "配置",
        headerKicker: "",
        headerTitle: "Origin管理端",
        headerDescription: "",
        summaryCache: "Token 命中",
        summaryUsers: "用户",
        summarySpend: "消费",
        summaryHealth: "状态",
        nominal: "正常",
        attention: "关注",
        overview: "概览",
        overviewSub: "缓存、额度与账号状态",
        cacheHit: "缓存命中",
        tokenHitRate: "Token 级命中率",
        prefixCache: "累计缓存 Input Tokens",
        totalInputTokens: "累计 Input Tokens",
        requestHitLabel: "请求级",
        tokenHitLabel: "Token 级",
        cacheProfileTitle: "缓存命中画像",
        cacheProfileDesc: "以 Token 命中为主，请求级仅作参考。",
        observedWindow: "统计窗口",
        observedWindowValue: "最近 512 条请求",
        recoveredInput: "已回收输入",
        ringCenterLabel: "Token 命中",
        cacheHitHint: "Token=缓存/输入 · 请求=命中请求占比",
        hit: "命中",
        tokens: "MToken",
        lowQuota: "低额度",
        lowQuotaDesc: "额度接近下限。",
        active: "活跃",
        activeDesc: "当前可路由账号。",
        protected: "保护中",
        protectedDesc: "保护已启用。",
        totalSpend: "总消费",
        spendDesc: "模型价格估算。",
        requestsOverTime: "请求趋势",
        requests: "请求",
        topUsers: "消费靠前用户",
        pricedRequests: "已计价请求",
        accounts: "账号",
        accountsSub: "支持模型、5H/7D 额度与路由状态",
        all: "全部",
        filterAvailable: "可用",
        filterInUse: "使用中",
        filterDisabled: "不可用",
        searchDisabled: "搜索稍后接入",
        quota: "额度",
        quota5h: "5H",
        quota7d: "7D",
        route: "出口",
        modelCount: "模型数",
        refreshModels: "刷新模型",
        refreshing: "刷新中...",
        modelsReady: "模型目录已更新",
        refreshQuota: "刷新额度",
        quotaRefreshFailed: "刷新额度失败",
        quotaRefreshed: (label: string) => `账号 ${label} 的额度已刷新`,
        addAccount: "添加账号",
        cleanupBanned: "删除封禁",
        cleaningBanned: "删除中...",
        cleanupFailed: "删除封禁账号失败",
        bannedDeleted: (count: number) => `已删除 ${count} 个封禁账号`,
        noBannedAccounts: "没有可删除的封禁账号",
        autoQuotaRefresh: "自动刷新额度",
        autoQuotaRefreshHint: "受管账号额度每 60 秒后台刷新一次。",
        accountMenu: "账号操作",
        copyAccount: "复制账号信息",
        deleteAccount: "删除账号",
        deleteFailed: "删除账号失败",
        deleting: "删除中...",
        accountDeleted: (label: string) => `账号 ${label} 已删除`,
        accountCopied: (label: string) => `已复制 ${label} 的账号信息`,
        confirmDeleteAccount: (label: string) =>
          `确认删除账号 ${label}？该操作会移除凭证、租约和受管状态。`,
        confirmDeleteBanned: "确认一键删除所有封禁账号？该操作不可撤销。",
        managedOnlyAccountAction: "仅受管 ChatGPT 账号支持刷新额度。",
        statusLabel: "状态",
        reasonLabel: "原因",
        errorLabel: "错误",
        statusActive: "正常",
        statusUnavailable: "不可用",
        statusBanned: "封禁",
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
        fullKeyOnce: "打开用户配置即可查看并复制完整 Key；用户列表仍只显示预览。",
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
        alertsSub: "账号低额度、保护与不可用信号",
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
        requestReasoningHelp: "是否覆盖下游传入推理强度",
        liveRefresh: "实时刷新",
        refreshOff: "关闭",
        refreshPaused: "后台暂停",
        refreshOffline: "离线暂停",
        lastRefreshed: "最后刷新",
        settingsPanel: "设置",
        quotaSource: "额度来源：受管账号快照",
        usageSource: "缓存/用量来源：网关请求观测",
        plan: "套餐",
        workspace: "工作区角色",
        refreshedAt: "刷新于",
        cooldownUntil: "冷却至",
        resetsAt: "重置于"
      }
    : {
        brandTitle: "Origin管理端",
        brandSubtitle: "",
        navOverview: "Overview",
        navAccounts: "Accounts",
        navUsers: "Users",
        navAlerts: "Alerts",
        navLogs: "Logs",
        navConfig: "Config",
        headerKicker: "",
        headerTitle: "Origin管理端",
        headerDescription: "",
        summaryCache: "Token Hit",
        summaryUsers: "Users",
        summarySpend: "Spend",
        summaryHealth: "Health",
        nominal: "Nominal",
        attention: "Attention",
        overview: "Overview",
        overviewSub: "Cache, quota, and account state",
        cacheHit: "Cache Hit",
        tokenHitRate: "Token-level Cache Hit Rate",
        prefixCache: "Cached Input Tokens",
        totalInputTokens: "Total Input Tokens",
        requestHitLabel: "Request-level",
        tokenHitLabel: "Token-level",
        cacheProfileTitle: "Cache Hit Fidelity",
        cacheProfileDesc: "Prioritize token hit; keep request hit as reference.",
        observedWindow: "Observed Window",
        observedWindowValue: "Last 512 logged requests",
        recoveredInput: "Recovered Input",
        ringCenterLabel: "Token Hit",
        cacheHitHint: "Token=cached/input · Request=hit-request share",
        hit: "hit",
        tokens: "MToken",
        lowQuota: "Low Quota",
        lowQuotaDesc: "Quota is nearing the lower bound.",
        active: "Active",
        activeDesc: "Accounts currently routable.",
        protected: "Protected",
        protectedDesc: "Protection is enabled.",
        totalSpend: "Total Spend",
        spendDesc: "Estimated from model pricing.",
        requestsOverTime: "Requests over time",
        requests: "Requests",
        topUsers: "Top spend users",
        pricedRequests: "Priced requests",
        accounts: "Accounts",
        accountsSub: "Supported models, 5H/7D quota, and route state",
        all: "All",
        filterAvailable: "Available",
        filterInUse: "In use",
        filterDisabled: "Disabled",
        searchDisabled: "Search coming next",
        quota: "Quota",
        quota5h: "5H",
        quota7d: "7D",
        route: "Route",
        modelCount: "Models",
        refreshModels: "Refresh Models",
        refreshing: "Refreshing...",
        modelsReady: "Model catalog updated",
        refreshQuota: "Refresh Quota",
        quotaRefreshFailed: "Failed to refresh quota",
        quotaRefreshed: (label: string) => `Quota refreshed for ${label}`,
        addAccount: "Add Account",
        cleanupBanned: "Delete Banned",
        cleaningBanned: "Deleting...",
        cleanupFailed: "Failed to delete banned accounts",
        bannedDeleted: (count: number) => `Deleted ${count} banned accounts`,
        noBannedAccounts: "No banned accounts to delete",
        autoQuotaRefresh: "Quota auto refresh",
        autoQuotaRefreshHint: "Managed account quota is refreshed in the background every 60 seconds.",
        accountMenu: "Account actions",
        copyAccount: "Copy account details",
        deleteAccount: "Delete Account",
        deleteFailed: "Failed to delete account",
        deleting: "Deleting...",
        accountDeleted: (label: string) => `Deleted account ${label}`,
        accountCopied: (label: string) => `Copied ${label}`,
        confirmDeleteAccount: (label: string) =>
          `Delete account ${label}? This removes credentials, leases, and managed state.`,
        confirmDeleteBanned: "Delete all banned accounts? This cannot be undone.",
        managedOnlyAccountAction: "Only managed ChatGPT accounts can refresh quota.",
        statusLabel: "Status",
        reasonLabel: "Reason",
        errorLabel: "Error",
        statusActive: "Active",
        statusUnavailable: "Unavailable",
        statusBanned: "Banned",
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
          "Open user settings to view and copy the full key. The user list still keeps a preview only.",
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
        alertsSub: "Low quota, protection, and availability signals",
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
        requestReasoningHelp: "Override the downstream reasoning effort",
        liveRefresh: "Live refresh",
        refreshOff: "Off",
        refreshPaused: "Paused in background",
        refreshOffline: "Paused offline",
        lastRefreshed: "Last refresh",
        settingsPanel: "Settings",
        quotaSource: "Quota source: managed account snapshot",
        usageSource: "Cache/usage source: gateway telemetry",
        plan: "Plan",
        workspace: "Workspace role",
        refreshedAt: "Refreshed",
        cooldownUntil: "Cooldown until",
        resetsAt: "Resets"
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
  const visibleGatewayKey = latestIssuedToken ?? user.token;
  const shellSnippet = `export OPENAI_API_BASE="${gatewayBaseUrl}"\nexport OPENAI_API_KEY="${visibleGatewayKey}"\nexport CODEX_AFFINITY_ID="${affinityValue}"`;
  const codexSnippet = `model_provider = "gateway"\nmodel = "${defaultModel || "gpt-5.4"}"\n\n[model_providers.gateway]\nname = "AI Pool Gateway"\nbase_url = "${gatewayBaseUrl}"\nenv_key = "OPENAI_API_KEY"\nenv_http_headers = { "x-codex-cli-affinity-id" = "CODEX_AFFINITY_ID" }`;
  const curlSnippet = `curl ${gatewayBaseUrl.replace(/\/v1$/, "")}/v1/models -H "Authorization: Bearer ${visibleGatewayKey}" -H "x-codex-cli-affinity-id: ${affinityValue}"`;
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
            value: formatMToken(tokenUsage, language)
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
                    value: visibleGatewayKey,
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
                <p className={cx("mt-3 text-xs leading-6", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {t.fullKeyOnce}
                </p>
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
                        value: formatMToken(instance.totalTokens, language)
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
  const [theme, setTheme] = useState<ThemeMode>("light");
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
  const [refreshingQuotaAccountId, setRefreshingQuotaAccountId] = useState<string | null>(null);
  const [deletingAccountId, setDeletingAccountId] = useState<string | null>(null);
  const [cleaningBannedAccounts, setCleaningBannedAccounts] = useState(false);
  const [openAccountMenuId, setOpenAccountMenuId] = useState<string | null>(null);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [savingUserId, setSavingUserId] = useState<string | null>(null);
  const [banner, setBanner] = useState<BannerState>(null);
  const [latestCreatedUser, setLatestCreatedUser] =
    useState<CreatedGatewayUser | null>(null);
  const [liveRefreshInterval, setLiveRefreshInterval] =
    useState<LiveRefreshInterval>(5000);
  const [lastLiveRefreshAt, setLastLiveRefreshAt] = useState(snapshot.refreshedAt);
  const [isDocumentHidden, setIsDocumentHidden] = useState(false);
  const [isNavigatorOnline, setIsNavigatorOnline] = useState(true);

  const t = translationFor(language);
  const isDark = theme === "dark";
  const effectiveSidebarExpanded = sidebarExpanded || sidebarHovered;
  const liveRefreshLabel =
    liveRefreshInterval === 0
      ? t.refreshOff
      : isDocumentHidden
        ? t.refreshPaused
        : !isNavigatorOnline
          ? t.refreshOffline
          : `${liveRefreshInterval / 1000}s`;

  useEffect(() => {
    setRuntimeSnapshot(snapshot);
    setLastLiveRefreshAt(snapshot.refreshedAt);
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
    const syncPageState = () => {
      setIsDocumentHidden(document.visibilityState === "hidden");
      setIsNavigatorOnline(navigator.onLine);
    };

    syncPageState();
    document.addEventListener("visibilitychange", syncPageState);
    window.addEventListener("online", syncPageState);
    window.addEventListener("offline", syncPageState);

    return () => {
      document.removeEventListener("visibilitychange", syncPageState);
      window.removeEventListener("online", syncPageState);
      window.removeEventListener("offline", syncPageState);
    };
  }, []);

  useEffect(() => {
    if (liveRefreshInterval === 0 || isDocumentHidden || !isNavigatorOnline) {
      return undefined;
    }

    let disposed = false;
    let timeoutId: number | null = null;
    let inFlight = false;
    let controller: AbortController | null = null;

    const schedule = () => {
      if (disposed) {
        return;
      }
      timeoutId = window.setTimeout(() => {
        void fetchLiveSnapshot();
      }, liveRefreshInterval);
    };

    const fetchLiveSnapshot = async () => {
      if (disposed || inFlight) {
        return;
      }
      inFlight = true;
      controller?.abort();
      controller = new AbortController();

      try {
        const response = await fetch("/api/dashboard/live", {
          cache: "no-store",
          signal: controller.signal
        });
        const payload = (await response.json().catch(() => null)) as
          | DashboardLiveSnapshot
          | { error?: { message?: string } }
          | null;

        if (!response.ok || !payload || Array.isArray(payload) || "error" in payload) {
          throw new Error(
            payload &&
              typeof payload === "object" &&
              !Array.isArray(payload) &&
              "error" in payload
              ? payload.error?.message ?? t.refreshFailed
              : t.refreshFailed
          );
        }
        const livePayload = payload as DashboardLiveSnapshot;

        setRuntimeSnapshot((current) => ({
          ...current,
          refreshedAt: livePayload.refreshedAt,
          cacheMetrics: livePayload.cacheMetrics,
          accounts: livePayload.accounts,
          leases: livePayload.leases,
          accountAlerts: livePayload.accountAlerts,
          requestLogs: livePayload.requestLogs,
          billing: livePayload.billing,
          modelCatalog: Array.from(
            new Set([
              ...current.modelCatalog,
              ...livePayload.accounts.flatMap((account) => account.models)
            ])
          ).sort(),
          counts: {
            ...current.counts,
            accounts: livePayload.accounts.length,
            activeLeases: livePayload.leases.length,
            warpAccounts: livePayload.accounts.filter(
              (account) => account.routeMode === "warp"
            ).length
          }
        }));
        setLastLiveRefreshAt(livePayload.refreshedAt);
      } catch (error) {
        if (!(error instanceof DOMException && error.name === "AbortError")) {
          setBanner((current) =>
            current?.tone === "error"
              ? current
              : {
                  tone: "error",
                  message: error instanceof Error ? error.message : t.refreshFailed
                }
          );
        }
      } finally {
        inFlight = false;
        schedule();
      }
    };

    void fetchLiveSnapshot();

    return () => {
      disposed = true;
      controller?.abort();
      if (timeoutId !== null) {
        window.clearTimeout(timeoutId);
      }
    };
  }, [isDocumentHidden, isNavigatorOnline, liveRefreshInterval, t.refreshFailed]);

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

  useEffect(() => {
    if (!openAccountMenuId) {
      return undefined;
    }

    const handlePointerDown = (event: MouseEvent) => {
      if (
        event.target instanceof Element &&
        event.target.closest("[data-account-menu-root='true']")
      ) {
        return;
      }
      setOpenAccountMenuId(null);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpenAccountMenuId(null);
      }
    };

    document.addEventListener("mousedown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [openAccountMenuId]);

  useEffect(() => {
    if (!isSettingsOpen) {
      return undefined;
    }

    const handlePointerDown = (event: MouseEvent) => {
      if (
        event.target instanceof Element &&
        event.target.closest("[data-settings-panel-root='true']")
      ) {
        return;
      }
      setIsSettingsOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setIsSettingsOpen(false);
      }
    };

    document.addEventListener("mousedown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [isSettingsOpen]);

  useEffect(() => {
    if (
      openAccountMenuId &&
      !runtimeSnapshot.accounts.some((account) => account.id === openAccountMenuId)
    ) {
      setOpenAccountMenuId(null);
    }
  }, [openAccountMenuId, runtimeSnapshot.accounts]);

  const lowQuotaAccounts = useMemo(
    () =>
      runtimeSnapshot.accounts.filter(
        (account) => account.status === "active" && accountKind(account) === "low"
      ),
    [runtimeSnapshot.accounts]
  );
  const activeAccounts = useMemo(
    () =>
      runtimeSnapshot.accounts.filter(
        (account) => account.status === "active" && accountKind(account) === "active"
      ),
    [runtimeSnapshot.accounts]
  );
  const protectedAccounts = useMemo(
    () =>
      runtimeSnapshot.accounts.filter(
        (account) => account.status === "active" && accountKind(account) === "protected"
      ),
    [runtimeSnapshot.accounts]
  );
  const bannedAccounts = useMemo(
    () =>
      runtimeSnapshot.accounts.filter((account) => account.status === "banned"),
    [runtimeSnapshot.accounts]
  );
  const leasedAccountIds = useMemo(
    () => new Set(runtimeSnapshot.leases.map((lease) => lease.accountId)),
    [runtimeSnapshot.leases]
  );

  const filteredAccounts = useMemo(() => {
    if (accountFilter === "all") {
      return runtimeSnapshot.accounts;
    }

    return runtimeSnapshot.accounts.filter((account) => {
      const state = accountOperationalState(account, leasedAccountIds);
      if (accountFilter === "available") {
        return state === "available";
      }
      if (accountFilter === "inUse") {
        return state === "inUse";
      }
      return state === "disabled";
    });
  }, [accountFilter, leasedAccountIds, runtimeSnapshot.accounts]);

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
  const requestCacheHit = requestHitRatio(runtimeSnapshot.cacheMetrics);
  const tokenCacheHit = tokenHitRatio(runtimeSnapshot.cacheMetrics);

  function accountStatusLabel(status: AccountRecord["status"]) {
    if (status === "banned") {
      return t.statusBanned;
    }
    if (status === "unavailable") {
      return t.statusUnavailable;
    }
    return t.statusActive;
  }

  function accountAlertLabel(alert: AccountAlertRecord) {
    if (alert.kind === "disabled") {
      return t.statusBanned;
    }
    if (alert.kind === "unavailable") {
      return t.statusUnavailable;
    }
    if (alert.kind === "protected") {
      return t.protected;
    }
    return t.lowQuota;
  }

  function accountAlertReason(alert: AccountAlertRecord) {
    if (alert.kind === "low_quota") {
      return `${t.quota} ${alert.quotaHeadroom !== null ? `${percent(alert.quotaHeadroom)}%` : "--"}`;
    }
    if (alert.kind === "protected") {
      return alert.cooldownLevel > 0 ? `${t.protected} · L${alert.cooldownLevel}` : t.protected;
    }
    if (alert.kind === "disabled") {
      return t.statusBanned;
    }
    if (alert.kind === "unavailable") {
      return t.statusUnavailable;
    }
    return alert.reason ?? "--";
  }

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
        throw new Error(readApiErrorMessage(payload, t.refreshFailed));
      }

      setRuntimeSnapshot((current) => nextSnapshotWithAccounts(current, payload));
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

  function replaceAccount(updatedAccount: AccountRecord) {
    setRuntimeSnapshot((current) =>
      nextSnapshotWithAccounts(
        current,
        current.accounts.map((account) =>
          account.id === updatedAccount.id ? updatedAccount : account
        )
      )
    );
  }

  function removeAccounts(accountIds: string[]) {
    const removed = new Set(accountIds);
    setRuntimeSnapshot((current) =>
      nextSnapshotWithAccounts(
        current,
        current.accounts.filter((account) => !removed.has(account.id))
      )
    );
  }

  async function handleRefreshAccountQuota(account: AccountRecord) {
    setOpenAccountMenuId(null);
    if (account.authMode !== "chatgpt") {
      setBanner({
        tone: "error",
        message: t.managedOnlyAccountAction
      });
      return;
    }

    try {
      setRefreshingQuotaAccountId(account.id);
      const response = await fetch(
        `/api/dashboard/accounts/${account.id}/quota/refresh`,
        {
          method: "POST"
        }
      );
      const payload = (await response.json().catch(() => null)) as
        | AccountRecord
        | { error?: { message?: string } }
        | null;

      if (!response.ok || !isAccountRecordPayload(payload)) {
        throw new Error(readApiErrorMessage(payload, t.quotaRefreshFailed));
      }

      replaceAccount(payload);
      setBanner({
        tone: "ok",
        message: t.quotaRefreshed(account.label || shortId(account.id))
      });
    } catch (error) {
      setBanner({
        tone: "error",
        message: error instanceof Error ? error.message : t.quotaRefreshFailed
      });
    } finally {
      setRefreshingQuotaAccountId(null);
    }
  }

  async function handleCopyAccountDetails(account: AccountRecord) {
    setOpenAccountMenuId(null);
    const payload = [
      accountPrimaryIdentifier(account),
      accountSecondaryIdentifier(account),
      shortId(account.id),
      account.chatgptEmail ?? "",
      account.baseUrl ?? "",
      account.models.join(", ")
    ]
      .filter(Boolean)
      .join("\n");

    try {
      await copyText(payload);
      setBanner({
        tone: "ok",
        message: t.accountCopied(account.label || shortId(account.id))
      });
    } catch (error) {
      setBanner({
        tone: "error",
        message: error instanceof Error ? error.message : t.copy
      });
    }
  }

  async function handleDeleteAccount(account: AccountRecord) {
    const displayLabel = account.label || shortId(account.id);
    setOpenAccountMenuId(null);
    if (!window.confirm(t.confirmDeleteAccount(displayLabel))) {
      return;
    }

    try {
      setDeletingAccountId(account.id);
      const response = await fetch(`/api/dashboard/accounts/${account.id}`, {
        method: "DELETE"
      });
      const payload = (await response.json().catch(() => null)) as
        | { error?: { message?: string } }
        | null;

      if (!response.ok) {
        throw new Error(readApiErrorMessage(payload, t.deleteFailed));
      }

      removeAccounts([account.id]);
      setBanner({
        tone: "ok",
        message: t.accountDeleted(displayLabel)
      });
    } catch (error) {
      setBanner({
        tone: "error",
        message: error instanceof Error ? error.message : t.deleteFailed
      });
    } finally {
      setDeletingAccountId(null);
    }
  }

  async function handleCleanupBannedAccounts() {
    if (bannedAccounts.length === 0) {
      setBanner({
        tone: "neutral",
        message: t.noBannedAccounts
      });
      return;
    }
    if (!window.confirm(t.confirmDeleteBanned)) {
      return;
    }

    try {
      setCleaningBannedAccounts(true);
      const response = await fetch("/api/dashboard/accounts/cleanup/banned", {
        method: "POST"
      });
      const payload = (await response.json().catch(() => null)) as
        | AccountCleanupResult
        | { error?: { message?: string } }
        | null;

      if (!response.ok || !isAccountCleanupResultPayload(payload)) {
        throw new Error(readApiErrorMessage(payload, t.cleanupFailed));
      }

      removeAccounts(payload.deletedAccountIds);
      setBanner({
        tone: "ok",
        message:
          payload.deleted > 0
            ? t.bannedDeleted(payload.deleted)
            : t.noBannedAccounts
      });
    } catch (error) {
      setBanner({
        tone: "error",
        message: error instanceof Error ? error.message : t.cleanupFailed
      });
    } finally {
      setCleaningBannedAccounts(false);
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
    <div className="space-y-5">
      <SectionCard
        actions={
          <button
            className={cx(
              "flex h-10 w-10 items-center justify-center rounded-[18px] text-white shadow-soft transition-all duration-200 hover:opacity-90",
              isDark ? "bg-sky-500" : "bg-zinc-900"
            )}
            onClick={() => setIsDrawerOpen(true)}
            type="button"
          >
            <Plus size={16} strokeWidth={iconStroke} />
          </button>
        }
        icon={LayoutDashboard}
        subtitle={t.overviewSub}
        theme={theme}
        title={t.overview}
      >
        <div className="grid gap-3.5 2xl:grid-cols-[minmax(0,1.56fr)_minmax(300px,0.9fr)]">
          <div className="apple-panel rounded-[30px] p-4 shadow-panel md:p-5">
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1.18fr)_minmax(264px,0.88fr)] xl:items-center">
              <div className="min-w-0 space-y-4">
                <div className="flex flex-wrap items-center gap-2.5">
                  <div
                    className={cx(
                      "flex h-10 w-10 items-center justify-center rounded-[16px] shadow-soft",
                      isDark ? "bg-white/[0.06] text-sky-200" : "bg-white text-sky-600"
                    )}
                  >
                    <Database size={16} strokeWidth={iconStroke} />
                  </div>
                  <div className="min-w-0">
                    <h3
                      className={cx(
                        "max-w-[16ch] break-keep font-semibold tracking-[-0.05em]",
                        isDark ? "text-zinc-50" : "text-zinc-950"
                      )}
                      style={{ fontSize: "clamp(1.35rem, 2.1vw, 2.35rem)" }}
                    >
                      {t.cacheHit}
                    </h3>
                  </div>
                </div>

                <p
                  className={cx(
                    "max-w-xl text-[13px] leading-6 md:text-sm",
                    isDark ? "text-zinc-400" : "text-zinc-600"
                  )}
                >
                  {t.cacheProfileDesc}
                </p>

                <div className="grid gap-3 sm:grid-cols-2">
                  {[
                    {
                      label: t.recoveredInput,
                      value: formatMToken(runtimeSnapshot.cacheMetrics.cachedTokens, language),
                      accent: "text-sky-500"
                    },
                    {
                      label: t.totalInputTokens,
                      value: formatMToken(runtimeSnapshot.cacheMetrics.replayTokens, language),
                      accent: isDark ? "text-zinc-100" : "text-zinc-900"
                    }
                  ].map((item) => (
                    <div
                      className="apple-subtle-panel min-w-0 rounded-[20px] px-4 py-3.5"
                      key={item.label}
                    >
                      <p
                        className={cx(
                          "text-[10px] uppercase tracking-[0.14em]",
                          isDark ? "text-zinc-500" : "text-zinc-400"
                        )}
                      >
                        {item.label}
                      </p>
                      <p
                        className={cx(
                          "mt-1.5 truncate text-[15px] font-medium tracking-[-0.02em]",
                          item.accent
                        )}
                      >
                        {item.value}
                      </p>
                    </div>
                  ))}
                </div>

                <div
                  className={cx(
                    "flex flex-wrap items-center gap-x-3 gap-y-1.5 text-[10px] leading-4",
                    isDark ? "text-zinc-500" : "text-zinc-500"
                  )}
                >
                  {[t.observedWindowValue, t.quotaSource, t.usageSource, t.cacheHitHint].map(
                    (item) => (
                      <span className="inline-flex items-center gap-1.5" key={item}>
                        <span
                          className={cx(
                            "h-1 w-1 rounded-full",
                            isDark ? "bg-white/20" : "bg-zinc-300"
                          )}
                        />
                        <span>{item}</span>
                      </span>
                    )
                  )}
                </div>
              </div>

              <div className="apple-subtle-panel rounded-[28px] p-4">
                <div className="relative mx-auto h-52 w-52 sm:h-56 sm:w-56">
                  <div
                    className={cx(
                      "absolute inset-8 rounded-full blur-3xl",
                      isDark ? "bg-sky-500/10" : "bg-sky-500/8"
                    )}
                  />
                  <svg className="relative h-full w-full -rotate-90" viewBox="0 0 160 160">
                    <circle
                      className={isDark ? "stroke-[#182133]" : "stroke-[#d5dbe6]"}
                      cx="80"
                      cy="80"
                      fill="none"
                      r="54"
                      strokeWidth="14"
                    />
                    <circle
                      className="stroke-[#0a84ff] transition-all duration-500"
                      cx="80"
                      cy="80"
                      fill="none"
                      r="54"
                      strokeLinecap="round"
                      strokeWidth="14"
                      style={ringSegmentStyle(tokenCacheHit, 54, 0, 1)}
                    />
                    <circle
                      className="stroke-[#30d158] transition-all duration-500"
                      cx="80"
                      cy="80"
                      fill="none"
                      r="54"
                      strokeLinecap="round"
                      strokeWidth="6"
                      style={ringSegmentStyle(requestCacheHit, 54, 0, 1)}
                    />
                  </svg>
                  <div className="absolute inset-0 flex items-center justify-center text-center">
                    <div className="space-y-1.5">
                      <p
                        className={cx(
                          "font-semibold tracking-[-0.05em]",
                          isDark ? "text-zinc-50" : "text-zinc-950"
                        )}
                        style={{ fontSize: "clamp(1.8rem, 3.3vw, 2.6rem)" }}
                      >
                        {percent(tokenCacheHit)}%
                      </p>
                      <p
                        className={cx(
                          "text-[10px] uppercase tracking-[0.16em]",
                          isDark ? "text-zinc-400" : "text-zinc-500"
                        )}
                      >
                        {t.ringCenterLabel}
                      </p>
                      <span
                        className={cx(
                          "inline-flex rounded-full px-2.5 py-1 text-[10px]",
                          isDark ? "bg-white/[0.06] text-zinc-300" : "bg-white/85 text-zinc-600"
                        )}
                      >
                        {t.requestHitLabel} {percent(requestCacheHit)}%
                      </span>
                    </div>
                  </div>
                </div>

                <div className="mt-4 space-y-2.5">
                  {[                    
                    {
                      label: t.tokenHitLabel,
                      value: `${percent(tokenCacheHit)}%`,
                      accent: "bg-[#0a84ff]"
                    },
                    {
                      label: t.requestHitLabel,
                      value: `${percent(requestCacheHit)}%`,
                      accent: "bg-[#30d158]"
                    }
                  ].map((item) => (
                    <div
                      className="flex items-center justify-between gap-3 rounded-[18px] px-1"
                      key={item.label}
                    >
                      <div className="flex min-w-0 items-center gap-3">
                        <span className={cx("h-2.5 w-2.5 rounded-full", item.accent)} />
                        <p
                          className={cx(
                            "truncate text-[13px]",
                            isDark ? "text-zinc-200" : "text-zinc-800"
                          )}
                        >
                          {item.label}
                        </p>
                      </div>
                      <span
                        className={cx(
                          "text-[13px] font-medium tracking-[-0.02em]",
                          isDark ? "text-zinc-50" : "text-zinc-900"
                        )}
                      >
                        {item.value}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          </div>

          <div className="grid gap-3 sm:grid-cols-2 2xl:grid-cols-2">
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
                className="apple-subtle-panel min-w-0 rounded-[22px] p-3.5 shadow-soft"
                key={item.label}
              >
                <div className="mb-3 flex items-center justify-between">
                  <div
                    className={cx(
                      "flex h-8 w-8 items-center justify-center rounded-[14px] shadow-soft",
                      item.iconClass
                    )}
                  >
                    <Icon size={15} strokeWidth={iconStroke} />
                  </div>
                  <span className={cx("text-[10px] font-medium uppercase tracking-[0.12em]", isDark ? "text-zinc-500" : "text-zinc-400")}>
                    {item.label}
                  </span>
                </div>
                <p className={cx("text-[24px] font-medium tracking-[-0.04em]", isDark ? "text-zinc-50" : "text-zinc-900")}>
                  {item.value}
                </p>
                <p className={cx("mt-1 max-w-[18ch] text-[10px] leading-4", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {item.desc}
                </p>
              </div>
            );
          })}
          </div>
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
              "h-[264px] rounded-[24px] p-2.5",
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
                {formatMToken(runtimeSnapshot.billing.totalTokens, language)} ·{" "}
                {formatMToken(runtimeSnapshot.billing.totalOutputTokens, language)} output
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
        <div className="flex flex-wrap items-center gap-2">
          <button
            className={cx(
              "inline-flex h-11 w-11 items-center justify-center rounded-2xl transition-all duration-200 ease-[cubic-bezier(0.25,0.1,0.25,1)] active:scale-[0.98]",
              isDark
                ? "bg-rose-500/12 text-rose-200 hover:bg-rose-500/18"
                : "bg-rose-50 text-rose-600 hover:bg-rose-100",
              (cleaningBannedAccounts || bannedAccounts.length === 0) &&
                "cursor-not-allowed opacity-60"
            )}
            disabled={cleaningBannedAccounts || bannedAccounts.length === 0}
            onClick={handleCleanupBannedAccounts}
            title={t.cleanupBanned}
            type="button"
          >
            <ShieldAlert size={16} strokeWidth={iconStroke} />
          </button>
          <button
            className={cx(
              "inline-flex h-11 w-11 items-center justify-center rounded-2xl transition-all duration-200 ease-[cubic-bezier(0.25,0.1,0.25,1)] active:scale-[0.98]",
              isDark
                ? "bg-white/[0.06] text-zinc-200 hover:bg-white/[0.1]"
                : "bg-zinc-100 text-zinc-600 hover:bg-zinc-200",
              refreshingModels && "opacity-70"
            )}
            disabled={refreshingModels}
            onClick={handleRefreshModels}
            title={refreshingModels ? t.refreshing : t.refreshModels}
            type="button"
          >
            <RefreshCw
              className={refreshingModels ? "animate-spin" : undefined}
              size={16}
              strokeWidth={iconStroke}
            />
          </button>
          <div
            className={cx(
              "apple-segmented inline-flex h-11 items-center gap-2 rounded-2xl px-3 text-xs",
              isDark ? "text-zinc-400" : "text-zinc-500"
            )}
            title={t.autoQuotaRefreshHint}
          >
            <Clock3 size={14} strokeWidth={iconStroke} />
            <span>60s</span>
          </div>
          <button
            className={cx(
              "inline-flex h-11 w-11 items-center justify-center rounded-2xl transition-all duration-200 ease-[cubic-bezier(0.25,0.1,0.25,1)] hover:opacity-90 active:scale-[0.98]",
              isDark ? "bg-zinc-100 text-zinc-950" : "bg-zinc-900 text-white"
            )}
            onClick={() => setIsAccountDrawerOpen(true)}
            title={t.addAccount}
            type="button"
          >
            <Plus size={16} strokeWidth={iconStroke} />
          </button>
        </div>
      }
      icon={Bot}
      subtitle={t.accountsSub}
      theme={theme}
      title={t.accounts}
    >
      <div className="mb-5 flex flex-wrap items-center justify-between gap-3">
        <div className={cx("apple-segmented inline-flex items-center gap-1 rounded-[22px] p-1.5")}>
          {[
            { id: "all" as const, icon: LayoutDashboard, title: t.all },
            { id: "available" as const, icon: CheckCircle2, title: t.filterAvailable },
            { id: "inUse" as const, icon: User, title: t.filterInUse },
            { id: "disabled" as const, icon: CircleX, title: t.filterDisabled }
          ].map((filter) => {
            const Icon = filter.icon;
            return (
              <button
                className={cx(
                  "flex h-10 w-10 items-center justify-center rounded-[16px] transition-all duration-200 ease-[cubic-bezier(0.25,0.1,0.25,1)] active:scale-[0.98]",
                  accountFilter === filter.id
                    ? isDark
                      ? "bg-zinc-100 text-zinc-950"
                      : "bg-zinc-900 text-white"
                    : isDark
                      ? "text-zinc-400 hover:bg-white/[0.06]"
                      : "text-zinc-500 hover:bg-zinc-100"
                )}
                key={filter.id}
                onClick={() => setAccountFilter(filter.id)}
                title={filter.title}
                type="button"
              >
                <Icon size={16} strokeWidth={iconStroke} />
              </button>
            );
          })}
        </div>

        <button
          className={cx(
            "apple-segmented inline-flex h-11 w-11 items-center justify-center rounded-2xl transition-all duration-200",
            isDark ? "text-zinc-400 hover:bg-white/[0.05]" : "text-zinc-500 hover:bg-white"
          )}
          disabled
          title={t.searchDisabled}
          type="button"
        >
          <Search size={16} strokeWidth={iconStroke} />
        </button>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
        {filteredAccounts.map((account) => {
          const kind = accountKind(account);
          const quota5hWindow = accountRateLimitWindow(account, "5h");
          const quota7dWindow = accountRateLimitWindow(account, "7d");
          const primaryId = accountPrimaryIdentifier(account);
          const secondaryId = accountSecondaryIdentifier(account);
          const statusTone = accountStatusTone(account);
          const menuBusy =
            refreshingQuotaAccountId === account.id || deletingAccountId === account.id;
          const operationalState = accountOperationalState(account, leasedAccountIds);
          const isManaged = account.authMode === "chatgpt";
          const availabilityTimestamp =
            account.availabilityState === "quota_exhausted"
              ? account.availabilityResetAt
              : account.cooldownUntil;
          const availabilityIcon =
            account.availabilityState === "quota_exhausted" ? Clock3 : Shield;
          const AvailabilityIcon = availabilityIcon;

          return (
            <article
              className={cx(
                "apple-panel group rounded-[28px] p-5 shadow-soft transition-all duration-200 ease-[cubic-bezier(0.25,0.1,0.25,1)] hover:-translate-y-0.5 hover:shadow-panel active:scale-[0.995]"
              )}
              key={account.id}
            >
              <div className="mb-4 flex items-start justify-between gap-3">
                <div className="min-w-0 flex flex-1 items-center gap-3">
                  <span
                    className={cx(
                      "flex h-11 w-11 shrink-0 items-center justify-center rounded-full shadow-soft",
                      isDark ? "bg-white/[0.06]" : "bg-white"
                    )}
                    title={isManaged ? "ChatGPT" : "API"}
                  >
                    {isManaged ? (
                      <Bot size={18} strokeWidth={iconStroke} />
                    ) : (
                      <KeyRound size={18} strokeWidth={iconStroke} />
                    )}
                  </span>
                  <div className="min-w-0 flex-1">
                    <span
                      className={cx(
                        "mb-2 inline-flex h-8 w-8 items-center justify-center rounded-full",
                        operationalState === "available"
                          ? isDark
                            ? "bg-emerald-500/12 text-emerald-300"
                            : "bg-emerald-50 text-emerald-600"
                          : operationalState === "inUse"
                            ? isDark
                              ? "bg-amber-500/12 text-amber-300"
                              : "bg-amber-50 text-amber-600"
                            : isDark
                              ? "bg-rose-500/12 text-rose-300"
                              : "bg-rose-50 text-rose-600"
                      )}
                      title={
                        operationalState === "available"
                          ? t.filterAvailable
                          : operationalState === "inUse"
                            ? t.filterInUse
                            : t.filterDisabled
                      }
                    >
                      {operationalState === "available" ? (
                        <CheckCircle2 size={14} strokeWidth={iconStroke} />
                      ) : operationalState === "inUse" ? (
                        <User size={14} strokeWidth={iconStroke} />
                      ) : (
                        <CircleX size={14} strokeWidth={iconStroke} />
                      )}
                    </span>
                    <p
                      className={cx(
                        "truncate text-[15px] font-medium tracking-[-0.03em]",
                        isDark ? "text-zinc-50" : "text-zinc-900"
                      )}
                    >
                      {primaryId}
                    </p>
                    <p
                      className={cx(
                        "mt-1 truncate text-xs",
                        isDark ? "text-zinc-500" : "text-zinc-500"
                      )}
                    >
                      {secondaryId}
                    </p>
                  </div>
                </div>
                <div className="relative" data-account-menu-root="true">
                  <button
                    aria-expanded={openAccountMenuId === account.id}
                    aria-haspopup="menu"
                    className={cx(
                      "flex h-9 w-9 items-center justify-center rounded-2xl transition-all duration-200",
                      isDark
                        ? "bg-white/[0.05] text-zinc-400 hover:bg-white/[0.08]"
                        : "bg-white text-zinc-500 hover:bg-zinc-100"
                    )}
                    onClick={() =>
                      setOpenAccountMenuId((current) =>
                        current === account.id ? null : account.id
                      )
                    }
                    title={t.accountMenu}
                    type="button"
                  >
                    <MoreHorizontal size={16} strokeWidth={iconStroke} />
                  </button>
                  {openAccountMenuId === account.id ? (
                    <div
                      className={cx(
                        "absolute right-0 top-11 z-20 rounded-[20px] border p-2 shadow-panel",
                        isDark
                          ? "border-white/10 bg-[#11141b]"
                          : "border-zinc-200 bg-white"
                      )}
                      role="menu"
                    >
                      <div className="flex items-center gap-1.5">
                        <button
                          className={cx(
                            "flex h-10 w-10 items-center justify-center rounded-2xl transition-all duration-200",
                            isDark
                              ? "text-zinc-200 hover:bg-white/[0.06]"
                              : "text-zinc-700 hover:bg-zinc-100"
                          )}
                          onClick={() => void handleCopyAccountDetails(account)}
                          title={t.copyAccount}
                          type="button"
                        >
                          <Copy size={14} strokeWidth={iconStroke} />
                        </button>
                        <button
                          className={cx(
                            "flex h-10 w-10 items-center justify-center rounded-2xl transition-all duration-200",
                            isDark
                              ? "text-zinc-200 hover:bg-white/[0.06]"
                              : "text-zinc-700 hover:bg-zinc-100",
                            (menuBusy || !isManaged) && "cursor-not-allowed opacity-45"
                          )}
                          disabled={menuBusy || !isManaged}
                          onClick={() => void handleRefreshAccountQuota(account)}
                          title={t.refreshQuota}
                          type="button"
                        >
                          <RefreshCw
                            className={
                              refreshingQuotaAccountId === account.id
                                ? "animate-spin"
                                : undefined
                            }
                            size={14}
                            strokeWidth={iconStroke}
                          />
                        </button>
                        <button
                          className={cx(
                            "flex h-10 w-10 items-center justify-center rounded-2xl transition-all duration-200",
                            isDark
                              ? "text-rose-200 hover:bg-rose-500/10"
                              : "text-rose-600 hover:bg-rose-50",
                            menuBusy && "cursor-not-allowed opacity-45"
                          )}
                          disabled={menuBusy}
                          onClick={() => void handleDeleteAccount(account)}
                          title={t.deleteAccount}
                          type="button"
                        >
                          <Trash2 size={14} strokeWidth={iconStroke} />
                        </button>
                      </div>
                    </div>
                  ) : null}
                </div>
              </div>

              <div className="space-y-3">
                {[
                  {
                    label: t.quota5h,
                    value: accountQuotaHeadroomLabel(account, "5h"),
                    ratio: clamp01(account.quotaHeadroom5h),
                    resetAt: quota5hWindow?.resetsAt ?? null
                  },
                  {
                    label: t.quota7d,
                    value: accountQuotaHeadroomLabel(account, "7d"),
                    ratio: clamp01(account.quotaHeadroom7d),
                    resetAt: quota7dWindow?.resetsAt ?? null
                  }
                ].map((windowQuota) => (
                  <div
                    className="space-y-1.5"
                    key={windowQuota.label}
                    title={
                      windowQuota.resetAt
                        ? `${t.resetsAt} ${formatDateTime(
                            new Date(windowQuota.resetAt * 1000).toISOString(),
                            language
                          )}`
                        : windowQuota.label
                    }
                  >
                    <div className={cx("flex items-center justify-between text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                      <span className="tracking-[0.16em]">{windowQuota.label}</span>
                      <span className={cx("font-medium tracking-[-0.01em]", isDark ? "text-zinc-200" : "text-zinc-800")}>
                        {windowQuota.value}%
                      </span>
                    </div>
                    <div className={cx("h-2.5 overflow-hidden rounded-full", isDark ? "bg-white/10" : "bg-zinc-200")}>
                      <div
                        className={cx("h-full rounded-full", quotaBarColor(windowQuota.ratio))}
                        style={{ width: `${windowQuota.value}%` }}
                      />
                    </div>
                  </div>
                ))}
              </div>

              <div className="mt-5 flex flex-wrap items-center gap-2">
                <span
                  className={cx(
                    "apple-segmented inline-flex h-9 items-center gap-2 rounded-full px-3 text-xs",
                    isDark ? "text-zinc-300" : "text-zinc-600"
                  )}
                  title={t.route}
                >
                  <ArrowUpRight size={13} strokeWidth={iconStroke} />
                  {account.currentMode}
                </span>
                <span
                  className={cx(
                    "apple-segmented inline-flex h-9 items-center gap-2 rounded-full px-3 text-xs",
                    isDark ? "text-zinc-300" : "text-zinc-600"
                  )}
                  title={t.modelCount}
                >
                  <Sparkles size={13} strokeWidth={iconStroke} />
                  {account.models.length}
                </span>
                {kind === "protected" ? (
                  <span
                    className={cx(
                      "inline-flex h-9 items-center gap-2 rounded-full px-3 text-xs",
                      isDark ? statusTone.chip : statusTone.chipLight
                    )}
                    title={
                      account.availabilityState === "quota_exhausted"
                        ? t.resetsAt
                        : t.protected
                    }
                  >
                    <AvailabilityIcon size={13} strokeWidth={iconStroke} />
                    {account.availabilityState === "quota_exhausted"
                      ? account.availabilityResetAt
                        ? formatDateTime(account.availabilityResetAt, language)
                        : t.lowQuota
                      : account.cooldownLevel > 0
                        ? `L${account.cooldownLevel}`
                        : t.protected}
                  </span>
                ) : null}
              </div>

              <div className="mt-4 flex items-center justify-between gap-3">
                <div className="flex min-w-0 flex-wrap gap-2">
                  {account.models.slice(0, 2).map((model) => (
                    <span
                      className={cx(
                        "truncate rounded-full px-3 py-1.5 text-[11px]",
                        isDark
                          ? "bg-white/[0.05] text-zinc-400"
                          : "bg-white text-zinc-500"
                      )}
                      key={model}
                      title={model}
                    >
                      {model}
                    </span>
                  ))}
                </div>
                {isManaged ? (
                  <span
                    className={cx(
                      "inline-flex shrink-0 items-center gap-1.5 text-[11px]",
                      isDark ? "text-zinc-500" : "text-zinc-500"
                    )}
                    title={t.autoQuotaRefreshHint}
                  >
                    <Clock3 size={12} strokeWidth={iconStroke} />
                    {account.managedStateRefreshedAt
                      ? relativeTime(account.managedStateRefreshedAt, language)
                      : `${managedQuotaRefreshIntervalMs / 1000}s`}
                  </span>
                ) : null}
              </div>

              {availabilityTimestamp ? (
                <p
                  className={cx(
                    "mt-3 text-[11px]",
                    account.availabilityState === "quota_exhausted"
                      ? isDark
                        ? "text-rose-300"
                        : "text-rose-600"
                      : isDark
                        ? "text-zinc-500"
                        : "text-zinc-500"
                  )}
                  title={account.availabilityReason ?? undefined}
                >
                  {account.availabilityState === "quota_exhausted"
                    ? `${t.resetsAt} ${formatDateTime(availabilityTimestamp, language)}`
                    : `${t.cooldownUntil} ${formatDateTime(availabilityTimestamp, language)}`}
                </p>
              ) : null}

              {account.lastError ? (
                <p
                  className={cx(
                    "mt-4 text-xs leading-5",
                    isDark ? "text-rose-300" : "text-rose-600"
                  )}
                  title={account.lastError}
                >
                  {truncateText(account.lastError, 92)}
                </p>
              ) : null}
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
                value: formatMToken(runtimeSnapshot.billing.totalTokens, language)
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
                          value: formatMToken(totalTokens, language)
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
      {runtimeSnapshot.accountAlerts.length === 0 ? (
        <EmptyState icon={BellOff} theme={theme} title={t.noAlerts} />
      ) : (
        <div className="space-y-4">
          {runtimeSnapshot.accountAlerts.map((alert) => (
            <div className="flex gap-4" key={alert.id}>
              <div className="flex flex-col items-center">
                <span
                  className={cx(
                    "flex h-10 w-10 items-center justify-center rounded-2xl shadow-soft",
                    alert.severity === "critical"
                      ? isDark
                        ? "bg-rose-500/14 text-rose-200"
                        : "bg-rose-50 text-rose-600"
                      : isDark
                        ? "bg-amber-500/14 text-amber-200"
                        : "bg-amber-50 text-amber-600"
                  )}
                >
                  {alert.kind === "disabled" ? (
                    <CircleX size={16} strokeWidth={iconStroke} />
                  ) : alert.kind === "protected" ? (
                    <Shield size={16} strokeWidth={iconStroke} />
                  ) : (
                    <Bell size={16} strokeWidth={iconStroke} />
                  )}
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
                  {accountAlertLabel(alert)} · {alert.accountLabel || shortId(alert.accountId)}
                </p>
                <p className={cx("mt-2 text-xs leading-5", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {accountAlertReason(alert)}
                </p>
                <p className={cx("mt-3 text-xs", isDark ? "text-zinc-600" : "text-zinc-400")}>
                  {relativeTime(alert.happenedAt, language)}
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
          "apple-shell fixed inset-y-4 left-4 z-40 hidden rounded-[30px] p-2 shadow-panel transition-all duration-200 xl:flex xl:flex-col",
          effectiveSidebarExpanded ? "w-[220px]" : "w-[64px]"
        )}
        onMouseEnter={() => setSidebarHovered(true)}
        onMouseLeave={() => setSidebarHovered(false)}
      >
        <div
          className={cx(
            "mb-4 flex items-center overflow-hidden",
            effectiveSidebarExpanded ? "gap-3" : "justify-center"
          )}
        >
          <div className="flex items-center gap-3 overflow-hidden">
            <span
              className={cx(
                "flex h-10 w-10 shrink-0 items-center justify-center rounded-[16px]",
                isDark ? "bg-zinc-100 text-zinc-950" : "bg-zinc-900 text-white"
              )}
            >
              <Bot size={16} strokeWidth={iconStroke} />
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
              {t.brandSubtitle ? (
                <p className={cx("text-xs", isDark ? "text-zinc-500" : "text-zinc-500")}>
                  {t.brandSubtitle}
                </p>
              ) : null}
            </div>
          </div>
        </div>

        <nav
          className={cx(
            "flex-1 space-y-1.5",
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
                "apple-subtle-panel rounded-[24px] p-2"
              )}
            >
              <div className="flex items-center justify-between gap-2">
                <span
                  className={cx(
                    "relative flex h-9 w-9 shrink-0 items-center justify-center rounded-[14px]",
                    isDark ? "bg-white/[0.06] text-zinc-300" : "bg-white text-zinc-500"
                  )}
                >
                  <Sparkles size={14} strokeWidth={iconStroke} />
                  <span
                    className={cx(
                      "absolute right-1.5 top-1.5 h-2 w-2 rounded-full ring-2",
                      health.status === "ok" ? "bg-emerald-400" : "bg-amber-400",
                      isDark ? "ring-[#141821]" : "ring-white"
                    )}
                  />
                </span>
                <div className="min-w-0">
                  <p className={cx("text-[11px] font-medium", isDark ? "text-zinc-100" : "text-zinc-900")}>
                    {health.status === "ok" ? t.systemReady : t.checkSystem}
                  </p>
                  <p className={cx("mt-0.5 text-[10px]", isDark ? "text-zinc-500" : "text-zinc-500")}>
                    {t.accountsInPool(runtimeSnapshot.counts.accounts)}
                  </p>
                </div>
                <button
                  className={cx(
                    "flex h-8 w-8 shrink-0 items-center justify-center rounded-[14px] transition-all duration-200",
                    isDark
                      ? "text-zinc-400 hover:bg-white/[0.06]"
                      : "text-zinc-500 hover:bg-white"
                  )}
                  onClick={() => setSidebarExpanded((value) => !value)}
                  title={t.collapse}
                  type="button"
                >
                  <ChevronLeft size={14} strokeWidth={iconStroke} />
                </button>
              </div>
            </div>
          ) : (
            <div
              className={cx(
                "apple-subtle-panel mx-auto flex w-11 flex-col items-center gap-1.5 rounded-[22px] p-1.5"
              )}
            >
              <span
                className={cx(
                  "relative flex h-8 w-8 items-center justify-center rounded-[14px]",
                  isDark ? "bg-white/[0.06] text-zinc-300" : "bg-white text-zinc-500"
                )}
                title={health.status === "ok" ? t.systemReady : t.checkSystem}
              >
                <Sparkles size={14} strokeWidth={iconStroke} />
                <span
                  className={cx(
                    "absolute right-1 top-1 h-2 w-2 rounded-full ring-2",
                    health.status === "ok" ? "bg-emerald-400" : "bg-amber-400",
                    isDark ? "ring-[#141821]" : "ring-white"
                  )}
                />
              </span>

              <div className={cx("h-px w-5", isDark ? "bg-white/10" : "bg-zinc-200")} />

              <button
                className={cx(
                  "flex h-8 w-8 items-center justify-center rounded-[14px] transition-all duration-200",
                  isDark
                    ? "bg-white/[0.06] text-zinc-300 hover:bg-white/[0.1]"
                    : "bg-white text-zinc-500 hover:bg-zinc-50"
                )}
                onClick={() => setSidebarExpanded((value) => !value)}
                title={t.expand}
                type="button"
              >
                <ChevronRight size={14} strokeWidth={iconStroke} />
              </button>
            </div>
          )}
        </div>
      </aside>

      <aside
        className={cx(
          "apple-shell fixed inset-x-4 bottom-4 z-40 rounded-[28px] p-3 shadow-panel xl:hidden"
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
          "min-h-screen p-3 pb-28 transition-all duration-200 md:p-5 md:pb-6",
          effectiveSidebarExpanded ? "xl:pl-[232px]" : "xl:pl-[88px]"
        )}
      >
        <div className="mx-auto max-w-[1640px] space-y-5">
          <header
            className={cx(
              "apple-shell rounded-[34px] p-4 shadow-soft md:p-5"
            )}
          >
            <div className="flex items-center justify-between gap-4">
              <h1
                className={cx(
                  "min-w-0 break-keep text-[clamp(1.7rem,2.8vw,2.7rem)] font-semibold tracking-[-0.05em]",
                  isDark ? "text-zinc-50" : "text-zinc-950"
                )}
              >
                {t.headerTitle}
              </h1>

              <div className="relative shrink-0" data-settings-panel-root="true">
                <button
                  aria-expanded={isSettingsOpen}
                  aria-haspopup="dialog"
                  className={cx(
                    "apple-segmented flex h-11 w-11 items-center justify-center rounded-2xl transition-all duration-200",
                    isDark
                      ? "text-zinc-200 hover:bg-white/[0.08]"
                      : "text-zinc-700 hover:bg-white"
                  )}
                  onClick={() => setIsSettingsOpen((current) => !current)}
                  title={t.settingsPanel}
                  type="button"
                >
                  <Settings size={17} strokeWidth={iconStroke} />
                </button>

                {isSettingsOpen ? (
                  <div
                    className={cx(
                      "apple-panel absolute right-0 top-14 z-30 w-[min(92vw,360px)] rounded-[28px] p-4 shadow-panel"
                    )}
                  >
                    <div className="space-y-4">
                      <div>
                        <p className={cx("text-sm font-semibold", isDark ? "text-zinc-100" : "text-zinc-900")}>
                          {t.settingsPanel}
                        </p>
                        <p className={cx("mt-1 text-[11px] leading-5", isDark ? "text-zinc-500" : "text-zinc-500")}>
                          {t.lastRefreshed}: {formatDateTime(lastLiveRefreshAt, language)} · {liveRefreshLabel}
                        </p>
                      </div>

                      <div className="space-y-2">
                        <p className={cx("text-[11px] tracking-[0.16em]", isDark ? "text-zinc-500" : "text-zinc-500")}>
                          {t.language}
                        </p>
                        <div className="apple-segmented inline-flex items-center gap-1 rounded-2xl p-1">
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
                      </div>

                      <div className="space-y-2">
                        <p className={cx("text-[11px] tracking-[0.16em]", isDark ? "text-zinc-500" : "text-zinc-500")}>
                          {t.liveRefresh}
                        </p>
                        <div className="apple-segmented flex flex-wrap items-center gap-1 rounded-2xl p-1">
                          {([
                            [5000, "5s"],
                            [10000, "10s"],
                            [30000, "30s"],
                            [0, t.refreshOff]
                          ] as const).map(([value, label]) => (
                            <button
                              className={cx(
                                "rounded-xl px-3 py-2 text-xs font-medium transition-all duration-200",
                                liveRefreshInterval === value
                                  ? isDark
                                    ? "bg-zinc-100 text-zinc-950"
                                    : "bg-zinc-900 text-white"
                                  : isDark
                                    ? "text-zinc-400 hover:bg-white/[0.06]"
                                    : "text-zinc-500 hover:bg-zinc-100"
                              )}
                              key={value}
                              onClick={() => setLiveRefreshInterval(value)}
                              type="button"
                            >
                              {label}
                            </button>
                          ))}
                        </div>
                      </div>

                      <button
                        className={cx(
                          "apple-segmented inline-flex min-h-12 w-full items-center justify-center gap-2 rounded-2xl px-4 py-3 text-sm font-medium transition-all duration-200",
                          isDark
                            ? "text-zinc-200 hover:bg-white/[0.08]"
                            : "text-zinc-700 hover:bg-white"
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
                    </div>
                  </div>
                ) : null}
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

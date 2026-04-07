import type { CSSProperties, ReactNode } from "react";
import { headers } from "next/headers";
import {
  browserRecoverAction
} from "@/app/actions";
import { AddAccountLauncher } from "@/components/account/AddAccountLauncher";
import { BrowserRecoverButton } from "@/components/buttons/BrowserRecoverButton";
import {
  getAdminHealth,
  getDashboardSnapshot,
  getTenants
} from "@/lib/dashboard";

export const dynamic = "force-dynamic";

const countFormat = new Intl.NumberFormat("zh-CN");
const timeFormat = new Intl.DateTimeFormat("zh-CN", {
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit"
});

type SearchParams =
  | Promise<Record<string, string | string[] | undefined>>
  | Record<string, string | string[] | undefined>
  | undefined;

type GlyphKind =
  | "brand"
  | "overview"
  | "cache"
  | "accounts"
  | "login"
  | "topology"
  | "leases"
  | "alerts"
  | "tasks"
  | "status"
  | "tenant"
  | "key";

function clamp01(value: number) {
  return Math.min(1, Math.max(0, value));
}

function percent(value: number) {
  return Math.round(clamp01(value) * 100);
}

function number(value: number) {
  return countFormat.format(value);
}

function formatTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "--";
  }
  return timeFormat.format(date);
}

function firstValue(
  value: string | string[] | undefined
) {
  return Array.isArray(value) ? value[0] : value;
}

function modeLabel(mode: string | null | undefined) {
  if (mode === "warp") {
    return "Warp";
  }
  if (mode === "direct") {
    return "Direct";
  }
  return "Hybrid";
}

function healthLabel(ok: boolean) {
  return ok ? "在线" : "离线";
}

function severityLabel(value: string) {
  const token = value.toLowerCase();
  if (token.includes("cool")) {
    return "冷却";
  }
  if (token.includes("critical")) {
    return "严重";
  }
  if (token.includes("warn")) {
    return "告警";
  }
  if (token.includes("recover")) {
    return "恢复";
  }
  return value;
}

function taskKindLabel(kind: string) {
  const labels: Record<string, string> = {
    login: "登录",
    recover: "恢复",
    profile: "配置",
    warmup: "预热",
    verify: "校验"
  };
  return labels[kind] ?? kind;
}

function taskStatusLabel(status: string) {
  const labels: Record<string, string> = {
    queued: "排队",
    running: "执行中",
    completed: "完成",
    failed: "失败",
    retrying: "重试"
  };
  return labels[status] ?? status;
}

function providerLabel(provider: string | null) {
  if (provider === "openai") {
    return "OpenAI";
  }
  if (!provider) {
    return "通用";
  }
  return provider;
}

function topologyLabel(name: string, purpose: string) {
  const labels: Record<string, string> = {
    web: "前端入口",
    "server:data": "数据网关",
    "server:admin": "控制面",
    "browser-assist": "浏览器辅助"
  };
  return labels[name] ?? purpose;
}

function topologyMark(name: string) {
  const labels: Record<string, string> = {
    web: "WE",
    "server:data": "GW",
    "server:admin": "AD",
    "browser-assist": "BA"
  };
  return labels[name] ?? "ND";
}

function signalClass(value: number) {
  if (value >= 0.8) {
    return "strong";
  }
  if (value >= 0.55) {
    return "steady";
  }
  if (value >= 0.3) {
    return "guarded";
  }
  return "weak";
}

function modelsLabel(models: string[]) {
  if (models.length <= 2) {
    return models.join(" / ");
  }
  return `${models.slice(0, 2).join(" / ")} +${models.length - 2}`;
}

function credentialLabel(
  hasCredential: boolean,
  baseUrl: string | null | undefined,
  chatgptAccountId: string | null | undefined
) {
  if (!hasCredential) {
    return "未绑定";
  }
  if (chatgptAccountId) {
    return chatgptAccountId;
  }
  if (baseUrl) {
    return baseUrl.replace(/^https?:\/\//, "");
  }
  return "已绑定";
}

function taskAssetLabel(task: {
  storageStatePath: string | null;
  screenshotPath: string | null;
}) {
  if (task.storageStatePath && task.screenshotPath) {
    return "State + Shot";
  }
  if (task.storageStatePath) {
    return "State";
  }
  if (task.screenshotPath) {
    return "Shot";
  }
  return "None";
}

function meterStyle(value: number) {
  return {
    "--meter": `${percent(value)}%`
  } as CSSProperties;
}

function Glyph({
  kind,
  className
}: {
  kind: GlyphKind;
  className?: string;
}) {
  let body: ReactNode;

  switch (kind) {
    case "brand":
      body = (
        <>
          <circle cx="18" cy="18" r="12" />
          <circle cx="18" cy="18" r="6" />
          <path d="M18 2v6M18 28v6M2 18h6M28 18h6" />
        </>
      );
      break;
    case "overview":
      body = (
        <>
          <rect x="5" y="6" width="10" height="10" rx="2" />
          <rect x="19" y="6" width="10" height="6" rx="2" />
          <rect x="19" y="16" width="10" height="12" rx="2" />
          <rect x="5" y="20" width="10" height="8" rx="2" />
        </>
      );
      break;
    case "cache":
      body = (
        <>
          <path d="M8 10h18v14H8z" />
          <path d="M11 7h12M11 28h12" />
          <path d="M14 14h6M14 18h8M14 22h4" />
        </>
      );
      break;
    case "accounts":
      body = (
        <>
          <circle cx="18" cy="13" r="5" />
          <path d="M8 29c1.8-5 6.4-8 10-8s8.2 3 10 8" />
        </>
      );
      break;
    case "login":
      body = (
        <>
          <path d="M15 8h11v20H15" />
          <path d="M9 18h14" />
          <path d="m17 12 6 6-6 6" />
        </>
      );
      break;
    case "topology":
      body = (
        <>
          <circle cx="9" cy="18" r="3" />
          <circle cx="27" cy="10" r="3" />
          <circle cx="27" cy="26" r="3" />
          <path d="M12 18h8M22 12l-6 4M22 24l-6-4" />
        </>
      );
      break;
    case "leases":
      body = (
        <>
          <path d="M13 22l-3 3a5 5 0 1 1-7-7l3-3" />
          <path d="M23 12l3-3a5 5 0 0 1 7 7l-3 3" />
          <path d="M12 24 24 12" />
        </>
      );
      break;
    case "alerts":
      body = (
        <>
          <path d="m18 6 13 24H5Z" />
          <path d="M18 13v7M18 24h.01" />
        </>
      );
      break;
    case "tasks":
      body = (
        <>
          <path d="M9 8h18v20H9z" />
          <path d="M13 12h10M13 18h10M13 24h6" />
          <path d="M5 12h2M5 18h2M5 24h2" />
        </>
      );
      break;
    case "status":
      body = (
        <>
          <path d="M8 25V11" />
          <path d="M18 25V7" />
          <path d="M28 25V15" />
        </>
      );
      break;
    case "tenant":
      body = (
        <>
          <path d="M6 13 18 6l12 7-12 7Z" />
          <path d="M10 17v7l8 4 8-4v-7" />
        </>
      );
      break;
    case "key":
      body = (
        <>
          <circle cx="12" cy="18" r="4" />
          <path d="M16 18h12M24 18v4M20 18v3" />
        </>
      );
      break;
    default:
      body = null;
  }

  return (
    <svg
      aria-hidden="true"
      className={className}
      fill="none"
      viewBox="0 0 36 36"
    >
      <g
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.75"
      >
        {body}
      </g>
    </svg>
  );
}

function Notice({
  tone,
  message
}: {
  tone: "ok" | "error";
  message: string;
}) {
  return (
    <section className={`notice ${tone}`}>
      <span className="notice-mark" aria-hidden="true">
        {tone === "ok" ? "OK" : "ER"}
      </span>
      <p>{message}</p>
    </section>
  );
}

function Empty({
  title,
  detail
}: {
  title: string;
  detail: string;
}) {
  return (
    <div className="empty-card">
      <strong>{title}</strong>
      <p>{detail}</p>
    </div>
  );
}

export default async function Page({
  searchParams
}: {
  searchParams?: SearchParams;
}) {
  const params = ((await searchParams) ??
    {}) as Record<string, string | string[] | undefined>;
  const noticeMessage = firstValue(params.noticeMessage);
  const noticeTone =
    firstValue(params.noticeTone) === "error" ? "error" : "ok";

  const [data, tenants, health] = await Promise.all([
    getDashboardSnapshot(),
    getTenants(),
    getAdminHealth()
  ]);
  const requestHeaders = await headers();
  const forwardedProto = requestHeaders.get("x-forwarded-proto");
  const forwardedHost =
    requestHeaders.get("x-forwarded-host") ?? requestHeaders.get("host");
  const webOrigin =
    forwardedHost
      ? `${forwardedProto ?? "http"}://${forwardedHost}`
      : "http://127.0.0.1:3000";
  const oauthCallbackUrl = `${webOrigin}/oauth/callback`;

  const surfaceTitle =
    data.title === "Codex Manager 2.0" ? "Codex 管理台" : data.title;
  const surfaceSubtitle =
    data.subtitle ===
    "Responses-first, lease-bound routing, dual-candidate selection, and warp-aware recovery."
      ? "响应优先、租约粘连、双候选调度、Warp 恢复。"
      : data.subtitle;

  const ringStyle = {
    "--progress": `${percent(data.cacheMetrics.prefixHitRatio)}%`
  } as CSSProperties;
  const generatedAt = formatTime(new Date().toISOString());

  const sidebarStatus = [
    {
      label: "管理面",
      value: healthLabel(health.status === "ok"),
      accent: health.status === "ok"
    },
    {
      label: "存储",
      value:
        health.storageMode === "postgres+memory"
          ? "双写"
          : health.storageMode === "memory-only"
            ? "内存"
            : "未知",
      accent: health.postgresConnected
    },
    {
      label: "Redis",
      value: health.redisConnected ? "在线" : "离线",
      accent: health.redisConnected
    },
    {
      label: "Browser",
      value: health.browserAssistUrl === "n/a" ? "未挂接" : "已挂接",
      accent: health.browserAssistUrl !== "n/a"
    }
  ];

  const summaryCards = [
    {
      glyph: "overview" as const,
      label: "租户",
      value: number(data.counts.tenants),
      note: "Tenant"
    },
    {
      glyph: "accounts" as const,
      label: "账号",
      value: number(data.counts.accounts),
      note: "Accounts"
    },
    {
      glyph: "leases" as const,
      label: "租约",
      value: number(data.counts.activeLeases),
      note: "Sticky"
    },
    {
      glyph: "tasks" as const,
      label: "任务",
      value: number(data.counts.browserTasks),
      note: "Browser"
    }
  ];

  const navItems = [
    { href: "#overview", label: "总览", glyph: "overview" as const },
    { href: "#connect", label: "接入", glyph: "key" as const },
    { href: "#accounts-ledger", label: "账号", glyph: "accounts" as const },
    { href: "#topology", label: "拓扑", glyph: "topology" as const },
    { href: "#leases", label: "租约", glyph: "leases" as const },
    { href: "#alerts", label: "告警", glyph: "alerts" as const },
    { href: "#tasks", label: "任务", glyph: "tasks" as const }
  ];

  return (
    <main className="console-shell">
      <div className="ambient ambient-a" />
      <div className="ambient ambient-b" />

      <aside className="chrome-sidebar">
        <div className="brand-card">
          <div className="brand-mark">
            <Glyph kind="brand" className="glyph" />
          </div>
          <div className="brand-copy">
            <strong>Codex Manager</strong>
            <span>Admin Surface / 中文默认</span>
          </div>
        </div>

        <nav className="nav-stack" aria-label="主导航">
          {navItems.map((item, index) => (
            <a className="nav-link" href={item.href} key={item.href}>
              <span className="nav-glyph">
                <Glyph kind={item.glyph} className="glyph" />
              </span>
              <span className="nav-copy">
                <strong>{item.label}</strong>
                <small>{String(index + 1).padStart(2, "0")}</small>
              </span>
            </a>
          ))}
        </nav>

        <section className="sidebar-panel">
          <header className="sidebar-head">
            <p className="section-kicker">Status Matrix</p>
            <h2>运行态</h2>
          </header>
          <div className="sidebar-status">
            {sidebarStatus.map((item) => (
              <div className="status-pill" key={item.label}>
                <span className={`status-dot ${item.accent ? "on" : "off"}`} />
                <div>
                  <strong>{item.label}</strong>
                  <small>{item.value}</small>
                </div>
              </div>
            ))}
          </div>
        </section>

        <section className="sidebar-panel slim">
          <header className="sidebar-head compact">
            <p className="section-kicker">Route</p>
            <h2>Warm Path</h2>
          </header>
          <div className="mini-metric-grid">
            <article className="mini-metric">
              <span>Hit</span>
              <strong>{percent(data.cacheMetrics.prefixHitRatio)}%</strong>
            </article>
            <article className="mini-metric">
              <span>Warp</span>
              <strong>{number(data.counts.warpAccounts)}</strong>
            </article>
          </div>
        </section>
      </aside>

      <div className="chrome-main">
        <header className="chrome-header">
          <div className="header-copy">
            <p className="eyebrow">Operations Console</p>
            <h1>{surfaceTitle}</h1>
            <p>{surfaceSubtitle}</p>
          </div>

          <div className="header-strip">
            <div className="header-chip strong">
              <Glyph kind="status" className="glyph chip-glyph" />
              <span>{healthLabel(health.status === "ok")}</span>
            </div>
            <div className="header-chip">
              <Glyph kind="cache" className="glyph chip-glyph" />
              <span>{percent(data.cacheMetrics.prefixHitRatio)}% 命中</span>
            </div>
            <div className="header-chip">
              <Glyph kind="tasks" className="glyph chip-glyph" />
              <span>{generatedAt}</span>
            </div>
          </div>
        </header>

        {noticeMessage ? (
          <Notice message={noticeMessage} tone={noticeTone} />
        ) : null}

        <section className="hero-grid" id="overview">
          <article className="glass-card hero-panel">
            <header className="panel-head hero-head">
              <div>
                <p className="section-kicker">Overview</p>
                <h2>控制台总览</h2>
              </div>
              <p className="panel-note">
                用更少的文字暴露关键状态，重点保留缓存、租约、账号和浏览器登录链路。
              </p>
            </header>

            <div className="hero-band">
              <div className="hero-band-copy">
                <strong>Responses-first / Sticky / Dual-Candidate</strong>
                <p>
                  入口、控制面、数据面、Browser Assist 分层展示；下方所有入口都保持中文默认。
                </p>
              </div>
              <div className="hero-band-tags">
                <span className="info-chip">CN</span>
                <span className="info-chip">OpenAI</span>
                <span className="info-chip">Docker</span>
                <span className="info-chip">Warp-aware</span>
              </div>
            </div>

            <div className="topology-rail">
              {data.topology.map((node, index) => (
                <div className="topology-node" key={node.name}>
                  <div className="topology-badge">{topologyMark(node.name)}</div>
                  <div className="topology-copy">
                    <strong>{topologyLabel(node.name, node.purpose)}</strong>
                    <small>{node.hotPath ? "Hot" : "Cold"}</small>
                  </div>
                  <code>:{node.port}</code>
                  {index < data.topology.length - 1 ? (
                    <div className="topology-line" aria-hidden="true" />
                  ) : null}
                </div>
              ))}
            </div>

            <div className="hero-stats">
              {summaryCards.map((item) => (
                <article className="summary-card" key={item.label}>
                  <span className="summary-glyph">
                    <Glyph kind={item.glyph} className="glyph" />
                  </span>
                  <div>
                    <strong>{item.value}</strong>
                    <span>{item.label}</span>
                  </div>
                  <small>{item.note}</small>
                </article>
              ))}
            </div>
          </article>

          <article className="glass-card cache-panel">
            <header className="panel-head compact">
              <div>
                <p className="section-kicker">Cache</p>
                <h2>缓存命中</h2>
              </div>
            </header>

            <div className="signal-ring" style={ringStyle}>
              <div className="signal-core">
                <strong>{percent(data.cacheMetrics.prefixHitRatio)}%</strong>
                <span>Prefix Hit</span>
              </div>
            </div>

            <div className="metric-stack">
              <article className="metric-block">
                <span>Cached Tokens</span>
                <strong>{number(data.cacheMetrics.cachedTokens)}</strong>
              </article>
              <article className="metric-block">
                <span>Replay Tokens</span>
                <strong>{number(data.cacheMetrics.replayTokens)}</strong>
              </article>
              <article className="metric-block">
                <span>Static Prefix</span>
                <strong>{number(data.cacheMetrics.staticPrefixTokens)}</strong>
              </article>
              <article className="metric-block">
                <span>Warmup ROI</span>
                <strong>{data.cacheMetrics.warmupRoi.toFixed(2)}x</strong>
              </article>
            </div>
          </article>
        </section>

        <section className="workspace-grid">
          <article className="glass-card intake-panel" id="connect">
            <header className="panel-head">
              <div>
                <p className="section-kicker">Intake</p>
                <h2>接入账号</h2>
              </div>
              <p className="panel-note">
                OpenAI 授权现在是主入口，会直接打开官方登录页；租户和 Token 导入保留为辅助工具。
              </p>
            </header>

            <div className="flow-rail compact">
              <div className="flow-step">
                <span>01</span>
                <strong>Login</strong>
                <small>登录导入</small>
              </div>
              <div className="flow-step">
                <span>02</span>
                <strong>Callback</strong>
                <small>回调解析</small>
              </div>
              <div className="flow-step">
                <span>03</span>
                <strong>Import</strong>
                <small>文件导入</small>
              </div>
            </div>

            <AddAccountLauncher
              accountCount={data.accounts.length}
              callbackUrl={oauthCallbackUrl}
              tenantCount={tenants.length}
            />

            <form className="form-card recover-card" action={browserRecoverAction}>
              <div className="form-head">
                <strong>
                  <Glyph kind="tasks" className="glyph inline-glyph" />
                  浏览器辅助恢复
                </strong>
                <span>保留 browser-assist 恢复入口，用于已有本地会话的恢复和截图校验。</span>
              </div>

              <div className="field-grid">
                <label className="field">
                  <span>账号</span>
                  <select
                    defaultValue={data.accounts[0]?.id ?? ""}
                    disabled={data.accounts.length === 0}
                    name="accountId"
                  >
                    {data.accounts.length === 0 ? (
                      <option value="">暂无账号</option>
                    ) : (
                      data.accounts.map((account) => (
                        <option key={account.id} value={account.id}>
                          {account.label} / {account.id}
                        </option>
                      ))
                    )}
                  </select>
                </label>
                <label className="field">
                  <span>路由</span>
                  <select defaultValue="direct" name="routeMode">
                    <option value="direct">Direct</option>
                    <option value="warp">Warp</option>
                  </select>
                </label>
                <label className="field">
                  <span>模式</span>
                  <select defaultValue="true" name="headless">
                    <option value="true">无头</option>
                    <option value="false">带界面</option>
                  </select>
                </label>
                <label className="field span-2">
                  <span>登录地址</span>
                  <input
                    autoComplete="off"
                    defaultValue="https://chatgpt.com/auth/login"
                    name="loginUrl"
                    type="text"
                  />
                </label>
                <label className="field span-2">
                  <span>备注</span>
                  <textarea
                    name="notes"
                    placeholder="例如：恢复本地浏览器状态并截图验证"
                    rows={3}
                  />
                </label>
              </div>

              <div className="form-actions">
                <BrowserRecoverButton disabled={data.accounts.length === 0} />
              </div>
            </form>
          </article>

          <article className="glass-card ledger-panel" id="accounts-ledger">
            <header className="panel-head">
              <div>
                <p className="section-kicker">Ledger</p>
                <h2>账号工作台</h2>
              </div>
              <p className="panel-note">
                表格化展示账号池，比大块卡片更适合看路由、配额和凭证状态。
              </p>
            </header>

            {data.accounts.length === 0 ? (
              <Empty detail="先导入账号，才能开启转发和登录任务。" title="暂无账号" />
            ) : (
              <div className="table-wrap">
                <table className="ledger-table">
                  <thead>
                    <tr>
                      <th>账号</th>
                      <th>模型</th>
                      <th>路由</th>
                      <th>H</th>
                      <th>Q</th>
                      <th>E</th>
                      <th>凭证</th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.accounts.map((account) => (
                      <tr key={account.id}>
                        <td>
                          <div className="table-primary">
                            <strong>{account.label}</strong>
                            <span>{account.id}</span>
                          </div>
                        </td>
                        <td>
                          <div className="table-secondary">
                            <strong>{modelsLabel(account.models)}</strong>
                            <span>
                              {account.nearQuotaGuardEnabled ? "Guard On" : "Guard Off"}
                            </span>
                          </div>
                        </td>
                        <td>
                          <div className="route-cell">
                            <span className="badge">{modeLabel(account.routeMode)}</span>
                            <span className="badge">L{account.cooldownLevel}</span>
                            <span className="badge">
                              {account.proxyEnabled ? "Proxy" : "Native"}
                            </span>
                          </div>
                        </td>
                        <td>
                          <div
                            className={`meter-card ${signalClass(account.healthScore)}`}
                            style={meterStyle(account.healthScore)}
                          >
                            <span>{percent(account.healthScore)}%</span>
                          </div>
                        </td>
                        <td>
                          <div
                            className={`meter-card ${signalClass(account.quotaHeadroom)}`}
                            style={meterStyle(account.quotaHeadroom)}
                          >
                            <span>{percent(account.quotaHeadroom)}%</span>
                          </div>
                        </td>
                        <td>
                          <div
                            className={`meter-card ${signalClass(account.egressStability)}`}
                            style={meterStyle(account.egressStability)}
                          >
                            <span>{percent(account.egressStability)}%</span>
                          </div>
                        </td>
                        <td>
                          <div className="table-secondary">
                            <strong>
                              {account.hasCredential ? "已绑定凭证" : "未绑定凭证"}
                            </strong>
                            <span>
                              {credentialLabel(
                                account.hasCredential,
                                account.baseUrl,
                                account.chatgptAccountId
                              )}
                            </span>
                          </div>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </article>
        </section>

        <section className="board-grid">
          <article className="glass-card" id="topology">
            <header className="panel-head compact">
              <div>
                <p className="section-kicker">Topology</p>
                <h2>拓扑</h2>
              </div>
              <p className="panel-note">热路径与冷路径分离</p>
            </header>
            <div className="stack">
              {data.topology.map((node) => (
                <div className="row-card topology-card" key={node.name}>
                  <div className="mark-block">{topologyMark(node.name)}</div>
                  <div className="row-copy">
                    <strong>{topologyLabel(node.name, node.purpose)}</strong>
                    <p>{node.name}</p>
                  </div>
                  <div className="row-meta">
                    <span>{node.hotPath ? "Hot" : "Cold"}</span>
                    <code>:{node.port}</code>
                  </div>
                </div>
              ))}
            </div>
          </article>

          <article className="glass-card" id="leases">
            <header className="panel-head compact">
              <div>
                <p className="section-kicker">Sticky Lease</p>
                <h2>租约</h2>
              </div>
              <p className="panel-note">principal 与 account 粘连</p>
            </header>
            <div className="stack">
              {data.leases.length === 0 ? (
                <Empty detail="当前没有活跃租约。" title="空闲" />
              ) : (
                data.leases.map((lease) => (
                  <div className="row-card" key={lease.principalId}>
                    <div className="row-copy">
                      <strong>{lease.accountLabel}</strong>
                      <p>{lease.principalId}</p>
                    </div>
                    <div className="badge-strip">
                      <span className="badge">{modeLabel(lease.routeMode)}</span>
                      <span className="badge">{lease.model}</span>
                      <span className="badge">G{lease.generation}</span>
                      <span className="badge">A{lease.activeSubagents}</span>
                      <span className="badge">{formatTime(lease.lastUsedAt)}</span>
                    </div>
                  </div>
                ))
              )}
            </div>
          </article>

          <article className="glass-card" id="alerts">
            <header className="panel-head compact">
              <div>
                <p className="section-kicker">Incidents</p>
                <h2>告警</h2>
              </div>
              <p className="panel-note">CF 压力与冷却等级</p>
            </header>
            <div className="stack">
              {data.cfIncidents.length === 0 ? (
                <Empty detail="当前没有 Cloudflare 压力事件。" title="平稳" />
              ) : (
                data.cfIncidents.map((incident) => (
                  <div className="row-card" key={incident.id}>
                    <div className="row-copy">
                      <strong>{incident.accountLabel}</strong>
                      <p>{incident.accountId}</p>
                    </div>
                    <div className="badge-strip">
                      <span className="badge">{modeLabel(incident.routeMode)}</span>
                      <span className="badge">{severityLabel(incident.severity)}</span>
                      <span className="badge">L{incident.cooldownLevel}</span>
                      <span className="badge">{formatTime(incident.happenedAt)}</span>
                    </div>
                  </div>
                ))
              )}
            </div>
          </article>
        </section>

        <section className="glass-card task-panel" id="tasks">
          <header className="panel-head">
            <div>
              <p className="section-kicker">Browser Tasks</p>
              <h2>浏览器任务</h2>
            </div>
            <p className="panel-note">
              登录、恢复、执行结果统一落在这里，便于判断 browser-assist 是否真正挂接。
            </p>
          </header>

          {data.browserTasks.length === 0 ? (
            <Empty detail="当前没有浏览器任务。" title="空队列" />
          ) : (
            <div className="table-wrap">
              <table className="ledger-table task-table">
                <thead>
                  <tr>
                    <th>任务</th>
                    <th>账号</th>
                    <th>Provider</th>
                    <th>Route</th>
                    <th>状态</th>
                    <th>资产</th>
                    <th>更新时间</th>
                  </tr>
                </thead>
                <tbody>
                  {data.browserTasks.map((task) => (
                    <tr key={task.id}>
                      <td>
                        <div className="table-primary">
                          <strong>{taskKindLabel(task.kind)}</strong>
                          <span>{task.id}</span>
                        </div>
                      </td>
                      <td>
                        <div className="table-secondary">
                          <strong>{task.accountLabel ?? "未绑定账号"}</strong>
                          <span>{task.accountId ?? "--"}</span>
                        </div>
                      </td>
                      <td>{providerLabel(task.provider)}</td>
                      <td>{modeLabel(task.routeMode)}</td>
                      <td>
                        <span className="badge">{taskStatusLabel(task.status)}</span>
                      </td>
                      <td>
                        <div className="table-secondary">
                          <strong>{taskAssetLabel(task)}</strong>
                          <span>S{task.stepCount}</span>
                        </div>
                      </td>
                      <td>{formatTime(task.updatedAt)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>
      </div>
    </main>
  );
}

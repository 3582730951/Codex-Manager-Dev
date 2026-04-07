import type { CSSProperties } from "react";
import {
  browserLoginAction,
  browserRecoverAction,
  createTenantAction,
  importAccountAction
} from "@/app/actions";
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
    return "◌ Warp";
  }
  if (mode === "direct") {
    return "◎ 直连";
  }
  return "○ 混合";
}

function healthLabel(ok: boolean) {
  return ok ? "在线" : "离线";
}

function severityLabel(value: string) {
  const token = value.toLowerCase();
  if (token.includes("cool")) {
    return "◍ 冷却";
  }
  if (token.includes("critical")) {
    return "◆ 严重";
  }
  if (token.includes("warn")) {
    return "△ 告警";
  }
  if (token.includes("recover")) {
    return "↗ 恢复";
  }
  return `○ ${value}`;
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
    web: "前台",
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

function Notice({
  tone,
  message
}: {
  tone: "ok" | "error";
  message: string;
}) {
  return (
    <section className={`notice ${tone}`}>
      <span className="notice-mark">{tone === "ok" ? "●" : "▲"}</span>
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

  const ringStyle = {
    "--progress": `${percent(data.cacheMetrics.prefixHitRatio)}%`
  } as CSSProperties;

  const stats = [
    { label: "租户", value: number(data.counts.tenants) },
    { label: "账号", value: number(data.counts.accounts) },
    { label: "租约", value: number(data.counts.activeLeases) },
    { label: "Warp", value: number(data.counts.warpAccounts) },
    { label: "任务", value: number(data.counts.browserTasks) }
  ];

  const healthBlocks = [
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
      value: health.redisConnected ? "已连通" : "未连通",
      accent: health.redisConnected
    },
    {
      label: "Browser",
      value: health.browserAssistUrl === "n/a" ? "未配置" : "已挂接",
      accent: health.browserAssistUrl !== "n/a"
    }
  ];

  return (
    <main className="shell">
      <div className="halo halo-a" />
      <div className="halo halo-b" />

      <section className="hero">
        <div className="hero-copy">
          <p className="eyebrow">Codex Manager / Admin Surface</p>
          <h1>{data.title}</h1>
          <p className="hero-subtitle">{data.subtitle}</p>

          <div className="status-strip">
            {healthBlocks.map((item) => (
              <div className="status-pill" key={item.label}>
                <span className={`status-dot ${item.accent ? "on" : "off"}`} />
                <strong>{item.label}</strong>
                <small>{item.value}</small>
              </div>
            ))}
          </div>
        </div>

        <div className="hero-signal">
          <div className="signal-ring" style={ringStyle}>
            <div className="signal-core">
              <strong>{percent(data.cacheMetrics.prefixHitRatio)}%</strong>
              <span>缓存命中</span>
            </div>
          </div>

          <div className="metric-stack">
            <article className="metric-block">
              <span>Cached</span>
              <strong>{number(data.cacheMetrics.cachedTokens)}</strong>
            </article>
            <article className="metric-block">
              <span>Replay</span>
              <strong>{number(data.cacheMetrics.replayTokens)}</strong>
            </article>
            <article className="metric-block">
              <span>Static</span>
              <strong>{number(data.cacheMetrics.staticPrefixTokens)}</strong>
            </article>
            <article className="metric-block">
              <span>ROI</span>
              <strong>{data.cacheMetrics.warmupRoi.toFixed(2)}x</strong>
            </article>
          </div>
        </div>
      </section>

      {noticeMessage ? (
        <Notice message={noticeMessage} tone={noticeTone} />
      ) : null}

      <section className="stats-grid">
        {stats.map((item, index) => (
          <article className="stat-card" key={item.label}>
            <small>{String(index + 1).padStart(2, "0")}</small>
            <strong>{item.value}</strong>
            <span>{item.label}</span>
          </article>
        ))}
      </section>

      <section className="action-grid">
        <article className="panel" id="connect">
          <header className="panel-head">
            <div>
              <p className="section-kicker">01</p>
              <h2>接入账号</h2>
            </div>
            <p className="panel-note">
              这里负责租户和上游凭证导入。用于网关真实转发。
            </p>
          </header>

          <div className="form-stack">
            <form action={createTenantAction} className="form-card">
              <div className="form-head">
                <strong>创建租户</strong>
                <span>先建租户，再导入账号</span>
              </div>
              <div className="field-grid compact">
                <label className="field">
                  <span>标识</span>
                  <input
                    autoComplete="off"
                    name="slug"
                    placeholder="demo-team"
                    type="text"
                  />
                </label>
                <label className="field">
                  <span>名称</span>
                  <input
                    autoComplete="off"
                    name="name"
                    placeholder="演示租户"
                    type="text"
                  />
                </label>
              </div>
              <div className="form-actions">
                <button className="button primary" type="submit">
                  创建租户
                </button>
              </div>
            </form>

            <form action={importAccountAction} className="form-card">
              <div className="form-head">
                <strong>导入账号</strong>
                <span>推荐填入 bearer token 与 ChatGPT Account ID</span>
              </div>

              <div className="field-grid">
                <label className="field">
                  <span>租户</span>
                  <select
                    defaultValue={tenants[0]?.id ?? ""}
                    disabled={tenants.length === 0}
                    name="tenantId"
                  >
                    {tenants.length === 0 ? (
                      <option value="">暂无租户</option>
                    ) : (
                      tenants.map((tenant) => (
                        <option key={tenant.id} value={tenant.id}>
                          {tenant.name} / {tenant.slug}
                        </option>
                      ))
                    )}
                  </select>
                </label>
                <label className="field">
                  <span>账号名</span>
                  <input
                    autoComplete="off"
                    name="label"
                    placeholder="OpenAI 主账号"
                    type="text"
                  />
                </label>
                <label className="field span-2">
                  <span>模型</span>
                  <input
                    autoComplete="off"
                    defaultValue="gpt-5.4, gpt-5.3-codex, gpt-5.2"
                    name="models"
                    type="text"
                  />
                </label>
                <label className="field span-2">
                  <span>Base URL</span>
                  <input
                    autoComplete="off"
                    defaultValue="https://chatgpt.com/backend-api/codex"
                    name="baseUrl"
                    type="text"
                  />
                </label>
                <label className="field span-2">
                  <span>Bearer Token</span>
                  <input
                    autoComplete="off"
                    name="bearerToken"
                    placeholder="sk-... / session token"
                    type="password"
                  />
                </label>
                <label className="field span-2">
                  <span>ChatGPT Account ID</span>
                  <input
                    autoComplete="off"
                    name="chatgptAccountId"
                    placeholder="acct_..."
                    type="text"
                  />
                </label>
                <label className="field span-2">
                  <span>额外请求头</span>
                  <textarea
                    name="extraHeaders"
                    placeholder={"Header-Name: value\nAnother-Header: value"}
                    rows={3}
                  />
                </label>
                <label className="field">
                  <span>Quota</span>
                  <input
                    inputMode="decimal"
                    name="quotaHeadroom"
                    placeholder="0.95"
                    type="text"
                  />
                </label>
                <label className="field">
                  <span>Quota 5h</span>
                  <input
                    inputMode="decimal"
                    name="quotaHeadroom5h"
                    placeholder="0.95"
                    type="text"
                  />
                </label>
                <label className="field">
                  <span>Quota 7d</span>
                  <input
                    inputMode="decimal"
                    name="quotaHeadroom7d"
                    placeholder="0.95"
                    type="text"
                  />
                </label>
                <label className="field">
                  <span>Health</span>
                  <input
                    inputMode="decimal"
                    name="healthScore"
                    placeholder="0.90"
                    type="text"
                  />
                </label>
                <label className="field">
                  <span>Egress</span>
                  <input
                    inputMode="decimal"
                    name="egressStability"
                    placeholder="0.88"
                    type="text"
                  />
                </label>
              </div>

              <div className="form-actions">
                <button
                  className="button primary"
                  disabled={tenants.length === 0}
                  type="submit"
                >
                  导入到控制面
                </button>
              </div>
            </form>
          </div>
        </article>

        <article className="panel" id="login">
          <header className="panel-head">
            <div>
              <p className="section-kicker">02</p>
              <h2>OpenAI 登录</h2>
            </div>
            <p className="panel-note">
              这里提交 browser-assist 任务，用于登录与恢复，不会替代 bearer token 导入。
            </p>
          </header>

          <form className="form-card tall" action={browserLoginAction}>
            <div className="form-head">
              <strong>浏览器辅助</strong>
              <span>
                Browser Assist: {health.browserAssistUrl === "n/a" ? "未挂接" : "已挂接"}
              </span>
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
                  <option value="direct">直连</option>
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
              <label className="field">
                <span>邮箱</span>
                <input autoComplete="username" name="email" type="text" />
              </label>
              <label className="field">
                <span>密码</span>
                <input autoComplete="current-password" name="password" type="password" />
              </label>
              <label className="field">
                <span>OTP</span>
                <input autoComplete="one-time-code" name="otpCode" type="text" />
              </label>
              <label className="field span-2">
                <span>备注</span>
                <textarea
                  name="notes"
                  placeholder="例如：首次登录、验证 Warp 会话"
                  rows={3}
                />
              </label>
            </div>

            <div className="form-actions dual">
              <button
                className="button primary"
                disabled={data.accounts.length === 0}
                type="submit"
              >
                启动登录
              </button>
              <button
                className="button ghost"
                disabled={data.accounts.length === 0}
                formAction={browserRecoverAction}
                type="submit"
              >
                恢复已有会话
              </button>
            </div>
          </form>
        </article>
      </section>

      <section className="content-grid">
        <article className="panel">
          <header className="panel-head compact">
            <div>
              <p className="section-kicker">03</p>
              <h2>拓扑</h2>
            </div>
            <p className="panel-note">热路径与冷路径分离</p>
          </header>

          <div className="stack">
            {data.topology.map((node) => (
              <div className="row-card topology" key={node.name}>
                <div className="mark-block">{topologyMark(node.name)}</div>
                <div className="row-copy">
                  <strong>{topologyLabel(node.name, node.purpose)}</strong>
                  <p>{node.name}</p>
                </div>
                <div className="row-meta">
                  <span>{node.hotPath ? "热" : "冷"}</span>
                  <code>:{node.port}</code>
                </div>
              </div>
            ))}
          </div>
        </article>

        <article className="panel">
          <header className="panel-head compact">
            <div>
              <p className="section-kicker">04</p>
              <h2>租约</h2>
            </div>
            <p className="panel-note">principal 与 account 粘连</p>
          </header>

          <div className="stack">
            {data.leases.length === 0 ? (
              <Empty
                detail="当前没有活跃租约。"
                title="空闲"
              />
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

        <article className="panel">
          <header className="panel-head compact">
            <div>
              <p className="section-kicker">05</p>
              <h2>账号池</h2>
            </div>
            <p className="panel-note">凭证、健康度、配额头寸</p>
          </header>

          <div className="stack">
            {data.accounts.length === 0 ? (
              <Empty detail="先导入账号，才能开启转发和登录任务。" title="暂无账号" />
            ) : (
              data.accounts.map((account) => (
                <div className="row-card account" key={account.id}>
                  <div className="row-copy">
                    <strong>{account.label}</strong>
                    <p>{account.id}</p>
                  </div>
                  <div className="account-signals">
                    <div className={`signal-tag ${signalClass(account.healthScore)}`}>
                      <small>Health</small>
                      <strong>{percent(account.healthScore)}</strong>
                    </div>
                    <div className={`signal-tag ${signalClass(account.quotaHeadroom)}`}>
                      <small>Quota</small>
                      <strong>{percent(account.quotaHeadroom)}</strong>
                    </div>
                    <div className={`signal-tag ${signalClass(account.egressStability)}`}>
                      <small>Egress</small>
                      <strong>{percent(account.egressStability)}</strong>
                    </div>
                  </div>
                  <div className="badge-strip">
                    <span className="badge">{modeLabel(account.routeMode)}</span>
                    <span className="badge">L{account.cooldownLevel}</span>
                    <span className="badge">
                      {account.hasCredential ? "已绑凭证" : "未绑凭证"}
                    </span>
                    <span className="badge">
                      {account.nearQuotaGuardEnabled ? "Guard On" : "Guard Off"}
                    </span>
                    <span className="badge">
                      {account.proxyEnabled ? "Proxy" : "Native"}
                    </span>
                  </div>
                </div>
              ))
            )}
          </div>
        </article>

        <article className="panel">
          <header className="panel-head compact">
            <div>
              <p className="section-kicker">06</p>
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

        <article className="panel full">
          <header className="panel-head compact">
            <div>
              <p className="section-kicker">07</p>
              <h2>浏览器任务</h2>
            </div>
            <p className="panel-note">登录、恢复、执行结果</p>
          </header>

          <div className="stack">
            {data.browserTasks.length === 0 ? (
              <Empty detail="当前没有浏览器任务。" title="空队列" />
            ) : (
              data.browserTasks.map((task) => (
                <div className="row-card" key={task.id}>
                  <div className="row-copy">
                    <strong>{taskKindLabel(task.kind)}</strong>
                    <p>{task.accountLabel ?? task.accountId ?? "未绑定账号"}</p>
                  </div>
                  <div className="badge-strip">
                    <span className="badge">{providerLabel(task.provider)}</span>
                    <span className="badge">{modeLabel(task.routeMode)}</span>
                    <span className="badge">{taskStatusLabel(task.status)}</span>
                    <span className="badge">S{task.stepCount}</span>
                    <span className="badge">{task.storageStatePath ? "State" : "No State"}</span>
                    <span className="badge">{task.screenshotPath ? "Shot" : "No Shot"}</span>
                    <span className="badge">{formatTime(task.updatedAt)}</span>
                  </div>
                </div>
              ))
            )}
          </div>
        </article>
      </section>
    </main>
  );
}

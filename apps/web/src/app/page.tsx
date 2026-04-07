import type { CSSProperties } from "react";
import { getDashboardSnapshot } from "@/lib/dashboard";

export const dynamic = "force-dynamic";

const countFormat = new Intl.NumberFormat("zh-CN");
const timeFormat = new Intl.DateTimeFormat("zh-CN", {
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit"
});

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

function modeLabel(mode: string | null | undefined) {
  if (mode === "warp") {
    return "🌀 Warp";
  }
  if (mode === "direct") {
    return "🟢 直连";
  }
  return "⚪ 混合";
}

function topologyIcon(name: string) {
  if (name === "web") {
    return "🖥️";
  }
  if (name === "server:data") {
    return "⚡";
  }
  if (name === "server:admin") {
    return "🎛️";
  }
  if (name === "browser-assist") {
    return "🤖";
  }
  return "◌";
}

function topologyLabel(name: string, purpose: string) {
  const labels: Record<string, string> = {
    web: "前台",
    "server:data": "网关",
    "server:admin": "管理",
    "browser-assist": "浏览器"
  };
  return labels[name] ?? purpose;
}

function severityLabel(severity: string) {
  const token = severity.toLowerCase();
  if (token.includes("cool")) {
    return "🧊 冷却";
  }
  if (token.includes("critical")) {
    return "🚨 严重";
  }
  if (token.includes("warn")) {
    return "🟠 告警";
  }
  if (token.includes("recover")) {
    return "🌤️ 恢复";
  }
  return `◌ ${severity}`;
}

function taskKindLabel(kind: string) {
  const labels: Record<string, string> = {
    login: "🔐 登录",
    recover: "🛟 恢复",
    profile: "👤 配置",
    warmup: "🔥 预热",
    verify: "✅ 校验"
  };
  return labels[kind] ?? `◌ ${kind}`;
}

function statusLabel(status: string) {
  const labels: Record<string, string> = {
    queued: "⏳ 排队",
    running: "🏃 执行中",
    completed: "✅ 完成",
    failed: "⚠️ 失败",
    retrying: "🔁 重试"
  };
  return labels[status] ?? `◌ ${status}`;
}

function providerLabel(provider: string | null) {
  if (!provider) {
    return "🧩 通用";
  }
  if (provider === "openai") {
    return "🧠 OpenAI";
  }
  return `🧩 ${provider}`;
}

function flagLabel(enabled: boolean, on: string, off: string) {
  return enabled ? on : off;
}

function meterLevel(value: number) {
  return Math.max(1, Math.round(clamp01(value) * 5));
}

function DotMeter({
  value,
  tone
}: {
  value: number;
  tone: "green" | "blue" | "amber" | "rose";
}) {
  const active = meterLevel(value);

  return (
    <span className={`mini-meter ${tone}`} aria-hidden="true">
      {Array.from({ length: 5 }, (_, index) => (
        <i className={index < active ? "on" : ""} key={index} />
      ))}
    </span>
  );
}

function Empty({
  icon,
  text
}: {
  icon: string;
  text: string;
}) {
  return (
    <div className="empty-state">
      <span>{icon}</span>
      <p>{text}</p>
    </div>
  );
}

export default async function Page() {
  const data = await getDashboardSnapshot();
  const ringStyle = {
    "--progress": `${percent(data.cacheMetrics.prefixHitRatio)}%`
  } as CSSProperties;
  const summaryCards = [
    { icon: "👥", label: "租户", value: data.counts.tenants, tone: "green" },
    { icon: "🪪", label: "账号", value: data.counts.accounts, tone: "blue" },
    { icon: "🔐", label: "租约", value: data.counts.activeLeases, tone: "amber" },
    { icon: "🌀", label: "Warp", value: data.counts.warpAccounts, tone: "rose" },
    { icon: "🤖", label: "任务", value: data.counts.browserTasks, tone: "green" }
  ];

  return (
    <main className="shell">
      <div className="ambient ambient-a" />
      <div className="ambient ambient-b" />

      <section className="hero-card">
        <div className="hero-copy">
          <div className="hero-badge">
            <span>🫧</span>
            <span>中文默认</span>
          </div>
          <h1>控制台</h1>
          <p>少字，多图，状态一眼看完。</p>
          <div className="hero-meta">
            <span>🧭 {data.title}</span>
            <span>⚙️ {data.subtitle}</span>
          </div>
        </div>

        <div className="hero-visual">
          <div className="signal-ring" style={ringStyle}>
            <div className="ring-core">
              <span>⚡</span>
              <strong>{percent(data.cacheMetrics.prefixHitRatio)}%</strong>
              <small>缓存命中</small>
            </div>
          </div>

          <div className="hero-metrics">
            <article className="metric-chip">
              <span>🧠</span>
              <strong>{number(data.cacheMetrics.cachedTokens)}</strong>
              <small>缓存</small>
            </article>
            <article className="metric-chip">
              <span>♻️</span>
              <strong>{number(data.cacheMetrics.replayTokens)}</strong>
              <small>回放</small>
            </article>
            <article className="metric-chip">
              <span>📦</span>
              <strong>{number(data.cacheMetrics.staticPrefixTokens)}</strong>
              <small>静态前缀</small>
            </article>
            <article className="metric-chip">
              <span>📈</span>
              <strong>{data.cacheMetrics.warmupRoi.toFixed(2)}x</strong>
              <small>ROI</small>
            </article>
          </div>
        </div>
      </section>

      <section className="count-grid">
        {summaryCards.map((card) => (
          <article className={`count-card ${card.tone}`} key={card.label}>
            <span className="count-icon">{card.icon}</span>
            <div>
              <strong>{number(card.value)}</strong>
              <small>{card.label}</small>
            </div>
          </article>
        ))}
      </section>

      <section className="content-grid">
        <article className="panel">
          <header className="panel-head">
            <div className="panel-title">
              <span>🧭</span>
              <strong>拓扑</strong>
            </div>
            <small>{data.topology.length} 点</small>
          </header>
          <div className="topology-grid">
            {data.topology.map((node) => (
              <div className={`node-card ${node.hotPath ? "hot" : ""}`} key={node.name}>
                <div className="node-symbol">{topologyIcon(node.name)}</div>
                <div className="node-copy">
                  <strong>{topologyLabel(node.name, node.purpose)}</strong>
                  <p>{node.name}</p>
                </div>
                <div className="node-meta">
                  <span>{node.hotPath ? "🔥" : "❄️"}</span>
                  <code>:{node.port}</code>
                </div>
              </div>
            ))}
          </div>
        </article>

        <article className="panel">
          <header className="panel-head">
            <div className="panel-title">
              <span>🔐</span>
              <strong>租约</strong>
            </div>
            <small>{data.leases.length} 条</small>
          </header>
          <div className="stack">
            {data.leases.length === 0 ? (
              <Empty icon="🌫️" text="当前没有活跃租约" />
            ) : (
              data.leases.map((lease) => (
                <div className="entity-row" key={lease.principalId}>
                  <div className="entity-main">
                    <strong>{lease.accountLabel}</strong>
                    <p>{lease.principalId}</p>
                  </div>
                  <div className="pill-rail">
                    <span className="chip">{modeLabel(lease.routeMode)}</span>
                    <span className="chip">🧠 {lease.model}</span>
                    <span className="chip">🧬 {lease.generation}</span>
                    <span className="chip">👥 {lease.activeSubagents}</span>
                    <span className="chip">🕒 {formatTime(lease.lastUsedAt)}</span>
                  </div>
                </div>
              ))
            )}
          </div>
        </article>

        <article className="panel">
          <header className="panel-head">
            <div className="panel-title">
              <span>🪪</span>
              <strong>账号</strong>
            </div>
            <small>{data.accounts.length} 个</small>
          </header>
          <div className="stack">
            {data.accounts.length === 0 ? (
              <Empty icon="🫥" text="当前没有账号数据" />
            ) : (
              data.accounts.map((account) => (
                <div className="entity-row" key={account.id}>
                  <div className="entity-main">
                    <strong>{account.label}</strong>
                    <p>{account.id}</p>
                  </div>
                  <div className="pill-rail compact">
                    <span className="chip signal-chip">
                      <span>🩺</span>
                      <DotMeter tone="green" value={account.healthScore} />
                      <b>{percent(account.healthScore)}</b>
                    </span>
                    <span className="chip signal-chip">
                      <span>📦</span>
                      <DotMeter tone="blue" value={account.quotaHeadroom} />
                      <b>{percent(account.quotaHeadroom)}</b>
                    </span>
                    <span className="chip signal-chip">
                      <span>🌐</span>
                      <DotMeter tone="amber" value={account.egressStability} />
                      <b>{percent(account.egressStability)}</b>
                    </span>
                    <span className="chip">{modeLabel(account.routeMode)}</span>
                    <span className="chip">🧊 L{account.cooldownLevel}</span>
                    <span className="chip">
                      {flagLabel(account.nearQuotaGuardEnabled, "🛡️", "⚪")} Guard
                    </span>
                    <span className="chip">
                      {flagLabel(account.hasCredential, "🔑", "⭕")} 凭证
                    </span>
                    <span className="chip">
                      {flagLabel(account.proxyEnabled, "🛰️", "🌤️")} 出口
                    </span>
                  </div>
                </div>
              ))
            )}
          </div>
        </article>

        <article className="panel">
          <header className="panel-head">
            <div className="panel-title">
              <span>🌀</span>
              <strong>告警</strong>
            </div>
            <small>{data.cfIncidents.length} 条</small>
          </header>
          <div className="stack">
            {data.cfIncidents.length === 0 ? (
              <Empty icon="🌤️" text="Cloudflare 压力正常" />
            ) : (
              data.cfIncidents.map((incident) => (
                <div className="entity-row" key={incident.id}>
                  <div className="entity-main">
                    <strong>{incident.accountLabel}</strong>
                    <p>{incident.accountId}</p>
                  </div>
                  <div className="pill-rail">
                    <span className="chip">{modeLabel(incident.routeMode)}</span>
                    <span className="chip">{severityLabel(incident.severity)}</span>
                    <span className="chip">🧊 L{incident.cooldownLevel}</span>
                    <span className="chip">🕒 {formatTime(incident.happenedAt)}</span>
                  </div>
                </div>
              ))
            )}
          </div>
        </article>

        <article className="panel full">
          <header className="panel-head">
            <div className="panel-title">
              <span>🤖</span>
              <strong>浏览器任务</strong>
            </div>
            <small>{data.browserTasks.length} 条</small>
          </header>
          <div className="stack">
            {data.browserTasks.length === 0 ? (
              <Empty icon="💤" text="当前没有浏览器辅助任务" />
            ) : (
              data.browserTasks.map((task) => (
                <div className="entity-row" key={task.id}>
                  <div className="entity-main">
                    <strong>{taskKindLabel(task.kind)}</strong>
                    <p>{task.accountLabel ?? task.accountId ?? "未绑定"}</p>
                  </div>
                  <div className="pill-rail">
                    <span className="chip">{providerLabel(task.provider)}</span>
                    <span className="chip">{modeLabel(task.routeMode)}</span>
                    <span className="chip">{statusLabel(task.status)}</span>
                    <span className="chip">👣 {task.stepCount}</span>
                    <span className="chip">
                      {flagLabel(Boolean(task.storageStatePath), "💾", "🫥")} 状态
                    </span>
                    <span className="chip">
                      {flagLabel(Boolean(task.screenshotPath), "🖼️", "◻️")} 截图
                    </span>
                    <span className="chip">🕒 {formatTime(task.updatedAt)}</span>
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

import { getDashboardSnapshot } from "@/lib/dashboard";

export const dynamic = "force-dynamic";

function ratio(value: number) {
  return `${Math.round(value * 100)}%`;
}

export default async function Page() {
  const data = await getDashboardSnapshot();

  return (
    <main className="shell">
      <div className="backdrop" />
      <section className="hero">
        <div className="hero-copy">
          <p className="eyebrow">Codex Manager / Control Surface</p>
          <h1>{data.title}</h1>
          <p className="lede">{data.subtitle}</p>
        </div>
        <div className="hero-stats">
          <article className="stat-card brass">
            <span>Tenants</span>
            <strong>{data.counts.tenants}</strong>
          </article>
          <article className="stat-card teal">
            <span>Accounts</span>
            <strong>{data.counts.accounts}</strong>
          </article>
          <article className="stat-card rust">
            <span>Active Leases</span>
            <strong>{data.counts.activeLeases}</strong>
          </article>
          <article className="stat-card slate">
            <span>Warp Accounts</span>
            <strong>{data.counts.warpAccounts}</strong>
          </article>
          <article className="stat-card ink">
            <span>Browser Tasks</span>
            <strong>{data.counts.browserTasks}</strong>
          </article>
        </div>
      </section>

      <section className="grid-panel">
        <article className="panel topology">
          <div className="panel-head">
            <span>Default Topology</span>
            <code>web / server / browser-assist</code>
          </div>
          <div className="topology-grid">
            {data.topology.map((node) => (
              <div className="topology-node" key={node.name}>
                <div className="node-title">
                  <strong>{node.name}</strong>
                  <span>{node.hotPath ? "hot-path" : "cold-path"}</span>
                </div>
                <p>{node.purpose}</p>
                <code>:{node.port}</code>
              </div>
            ))}
          </div>
        </article>

        <article className="panel cache-panel">
          <div className="panel-head">
            <span>Cache Yield</span>
            <code>static-first / replay-aware</code>
          </div>
          <div className="metric-rail">
            <div>
              <span>Cached Tokens</span>
              <strong>{data.cacheMetrics.cachedTokens.toLocaleString()}</strong>
            </div>
            <div>
              <span>Replay Tokens</span>
              <strong>{data.cacheMetrics.replayTokens.toLocaleString()}</strong>
            </div>
            <div>
              <span>Prefix Hit Ratio</span>
              <strong>{ratio(data.cacheMetrics.prefixHitRatio)}</strong>
            </div>
            <div>
              <span>Warmup ROI</span>
              <strong>{data.cacheMetrics.warmupRoi.toFixed(2)}x</strong>
            </div>
          </div>
          <div className="bar-shell">
            <div
              className="bar-fill"
              style={{ width: ratio(data.cacheMetrics.prefixHitRatio) }}
            />
          </div>
          <p className="muted">
            Static prefix budget:{" "}
            <code>{data.cacheMetrics.staticPrefixTokens.toLocaleString()} tokens</code>
          </p>
        </article>
      </section>

      <section className="grid-panel lower">
        <article className="panel">
          <div className="panel-head">
            <span>Lease Board</span>
            <code>single principal / single account</code>
          </div>
          <div className="lease-list">
            {data.leases.map((lease) => (
              <div className="lease-row" key={lease.principalId}>
                <div>
                  <p className="lease-title">{lease.accountLabel}</p>
                  <p className="lease-principal">{lease.principalId}</p>
                </div>
                <div className="lease-meta">
                  <span>{lease.model}</span>
                  <span>{lease.routeMode}</span>
                  <span>gen {lease.generation}</span>
                  <span>{lease.activeSubagents} agents</span>
                </div>
              </div>
            ))}
          </div>
        </article>

        <article className="panel">
          <div className="panel-head">
            <span>CF Pressure</span>
            <code>direct -&gt; warp -&gt; cooldown</code>
          </div>
          <div className="incident-list">
            {data.cfIncidents.map((incident) => (
              <div className="incident-row" key={incident.id}>
                <div>
                  <p className="lease-title">{incident.accountLabel}</p>
                  <p className="lease-principal">{incident.accountId}</p>
                </div>
                <div className="incident-meta">
                  <span>{incident.routeMode}</span>
                  <span>{incident.severity}</span>
                  <span>cooldown L{incident.cooldownLevel}</span>
                </div>
              </div>
            ))}
          </div>
        </article>
      </section>

      <section className="grid-panel lower">
        <article className="panel">
          <div className="panel-head">
            <span>Account Pool</span>
            <code>credential / health / cooldown</code>
          </div>
          <div className="account-list">
            {data.accounts.map((account) => (
              <div className="account-row" key={account.id}>
                <div>
                  <p className="lease-title">{account.label}</p>
                  <p className="lease-principal">{account.id}</p>
                </div>
                <div className="account-meta">
                  <span>{account.routeMode}</span>
                  <span>quota {ratio(account.quotaHeadroom)}</span>
                  <span>5h {ratio(account.quotaHeadroom5h)}</span>
                  <span>7d {ratio(account.quotaHeadroom7d)}</span>
                  <span>{account.nearQuotaGuardEnabled ? "guard-on" : "guard-off"}</span>
                  <span>health {ratio(account.healthScore)}</span>
                  <span>egress {ratio(account.egressStability)}</span>
                  <span>{account.egressGroup}</span>
                  <span>{account.proxyEnabled ? "proxy-on" : "proxy-off"}</span>
                  <span>cooldown L{account.cooldownLevel}</span>
                  <span>{account.hasCredential ? "credentialed" : "unbound"}</span>
                </div>
              </div>
            ))}
          </div>
        </article>

        <article className="panel">
          <div className="panel-head">
            <span>Browser Assist</span>
            <code>login / recover / profile</code>
          </div>
          <div className="incident-list">
            {data.browserTasks.map((task) => (
              <div className="incident-row" key={task.id}>
                <div>
                  <p className="lease-title">{task.kind}</p>
                  <p className="lease-principal">
                    {task.accountLabel ?? task.accountId ?? "unbound"}
                  </p>
                </div>
                <div className="incident-meta">
                  <span>{task.provider ?? "generic"}</span>
                  <span>{task.routeMode ?? "mixed"}</span>
                  <span>{task.status}</span>
                  <span>{task.stepCount} steps</span>
                  <span>{task.storageStatePath ? "state" : "no-state"}</span>
                  <span>{task.screenshotPath ? "shot" : "no-shot"}</span>
                </div>
              </div>
            ))}
          </div>
        </article>
      </section>
    </main>
  );
}

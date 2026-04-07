create table if not exists tenants (
    id uuid primary key,
    slug text not null unique,
    name text not null,
    created_at timestamptz not null
);

-- statement-break
create table if not exists gateway_api_keys (
    id uuid primary key,
    tenant_id uuid not null references tenants(id) on delete cascade,
    name text not null,
    token text not null unique,
    created_at timestamptz not null
);

-- statement-break
create table if not exists upstream_accounts (
    id text primary key,
    tenant_id uuid not null references tenants(id) on delete cascade,
    label text not null,
    models jsonb not null,
    current_mode text not null,
    signals jsonb not null,
    created_at timestamptz not null
);

-- statement-break
create table if not exists account_route_states (
    account_id text primary key references upstream_accounts(id) on delete cascade,
    route_mode text not null,
    direct_cf_streak integer not null,
    warp_cf_streak integer not null,
    cooldown_level integer not null,
    cooldown_until timestamptz null,
    warp_entered_at timestamptz null,
    last_cf_at timestamptz null,
    success_streak integer not null,
    last_success_at timestamptz null
);

-- statement-break
create table if not exists cli_leases (
    principal_id text primary key,
    tenant_id uuid not null references tenants(id) on delete cascade,
    account_id text not null references upstream_accounts(id) on delete cascade,
    account_label text not null,
    model text not null,
    reasoning_effort text null,
    route_mode text not null,
    generation integer not null,
    active_subagents integer not null,
    created_at timestamptz not null,
    last_used_at timestamptz not null
);

-- statement-break
create index if not exists idx_cli_leases_tenant_id on cli_leases (tenant_id);

-- statement-break
create table if not exists cf_incidents (
    id text primary key,
    account_id text not null references upstream_accounts(id) on delete cascade,
    account_label text not null,
    route_mode text not null,
    severity text not null,
    happened_at timestamptz not null,
    cooldown_level integer not null
);

-- statement-break
create index if not exists idx_cf_incidents_happened_at on cf_incidents (happened_at desc);

-- statement-break
create table if not exists cache_metrics (
    metric_key text primary key,
    cached_tokens bigint not null,
    replay_tokens bigint not null,
    prefix_hit_ratio double precision not null,
    warmup_roi double precision not null,
    static_prefix_tokens bigint not null,
    updated_at timestamptz not null
);

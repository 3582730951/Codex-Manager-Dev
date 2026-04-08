alter table gateway_api_keys
    add column if not exists email text not null default '';

-- statement-break
alter table gateway_api_keys
    add column if not exists role text not null default 'viewer';

-- statement-break
alter table gateway_api_keys
    add column if not exists default_model text null;

-- statement-break
alter table gateway_api_keys
    add column if not exists reasoning_effort text null;

-- statement-break
alter table gateway_api_keys
    add column if not exists force_model_override boolean not null default false;

-- statement-break
alter table gateway_api_keys
    add column if not exists force_reasoning_effort boolean not null default false;

-- statement-break
alter table gateway_api_keys
    add column if not exists updated_at timestamptz not null default now();

-- statement-break
create table if not exists request_logs (
    id text primary key,
    api_key_id uuid not null references gateway_api_keys(id) on delete cascade,
    tenant_id uuid not null references tenants(id) on delete cascade,
    user_name text not null,
    user_email text not null default '',
    principal_id text not null,
    account_id text not null,
    account_label text not null,
    method text not null,
    endpoint text not null,
    requested_model text not null,
    effective_model text not null,
    reasoning_effort text null,
    route_mode text not null,
    status_code integer not null,
    usage jsonb not null default '{}'::jsonb,
    estimated_cost_usd double precision null,
    created_at timestamptz not null
);

-- statement-break
create index if not exists idx_request_logs_created_at on request_logs (created_at desc);

-- statement-break
create index if not exists idx_request_logs_api_key_id on request_logs (api_key_id, created_at desc);

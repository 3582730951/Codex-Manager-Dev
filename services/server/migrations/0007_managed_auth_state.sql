alter table if exists upstream_credentials
    add column if not exists managed_auth_state jsonb null;

-- statement-break
alter table if exists account_route_states
    add column if not exists cooldown_reason text null;

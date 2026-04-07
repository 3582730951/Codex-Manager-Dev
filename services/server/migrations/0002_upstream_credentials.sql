create table if not exists upstream_credentials (
    account_id text primary key references upstream_accounts(id) on delete cascade,
    base_url text not null,
    bearer_token text not null,
    chatgpt_account_id text null,
    extra_headers jsonb not null default '[]'::jsonb,
    created_at timestamptz not null,
    updated_at timestamptz not null
);

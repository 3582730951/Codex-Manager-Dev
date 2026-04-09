create table if not exists conversation_threads (
  thread_id text primary key,
  tenant_id uuid not null references tenants(id) on delete cascade,
  principal_id text not null unique,
  root_thread_id text not null,
  parent_thread_id text,
  title text,
  model text,
  source text not null default 'gateway',
  status text not null default 'active',
  compaction_summary text,
  last_compaction_at timestamptz,
  created_at timestamptz not null,
  updated_at timestamptz not null
);

-- statement-break
create index if not exists idx_conversation_threads_tenant_updated_at
  on conversation_threads (tenant_id, updated_at desc);

-- statement-break
create table if not exists conversation_thread_edges (
  parent_thread_id text not null,
  child_thread_id text not null,
  relation text not null default 'fork',
  created_at timestamptz not null,
  primary key (parent_thread_id, child_thread_id, relation),
  foreign key (parent_thread_id) references conversation_threads(thread_id) on delete cascade,
  foreign key (child_thread_id) references conversation_threads(thread_id) on delete cascade
);

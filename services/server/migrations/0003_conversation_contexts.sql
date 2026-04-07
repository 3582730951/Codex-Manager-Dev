create table if not exists conversation_contexts (
  principal_id text primary key,
  model text not null,
  workflow_spine text not null,
  turns jsonb not null default '[]'::jsonb,
  updated_at timestamptz
);

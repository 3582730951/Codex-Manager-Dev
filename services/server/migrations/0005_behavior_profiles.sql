alter table conversation_contexts
  add column if not exists behavior_profile jsonb not null default '{"executionMode":"balanced","toolPolicy":"auto","verbosityPolicy":"normal","sessionEpoch":1}'::jsonb;

-- statement-break
alter table conversation_contexts
  add column if not exists pending_turn jsonb;

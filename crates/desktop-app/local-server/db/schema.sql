-- Local deployments (L2 instances managed by Desktop)
CREATE TABLE IF NOT EXISTS deployments (
  id TEXT PRIMARY KEY,
  program_slug TEXT NOT NULL DEFAULT 'evm-l2',
  name TEXT NOT NULL,
  chain_id INTEGER,
  rpc_url TEXT,
  status TEXT DEFAULT 'configured',
  deploy_method TEXT DEFAULT 'docker',
  config TEXT,
  host_id TEXT REFERENCES hosts(id),
  docker_project TEXT,
  deploy_dir TEXT,
  l1_port INTEGER,
  l2_port INTEGER,
  proof_coord_port INTEGER,
  phase TEXT DEFAULT 'configured',
  bridge_address TEXT,
  proposer_address TEXT,
  timelock_address TEXT,
  sp1_verifier_address TEXT,
  error_message TEXT,
  tools_l1_explorer_port INTEGER,
  tools_l2_explorer_port INTEGER,
  tools_bridge_ui_port INTEGER,
  tools_db_port INTEGER,
  tools_metrics_port INTEGER,
  env_project_id TEXT,
  env_updated_at INTEGER,
  is_public INTEGER DEFAULT 0,
  hashtags TEXT,
  created_at INTEGER NOT NULL
);

-- Remote hosts (SSH servers for remote deployment)
CREATE TABLE IF NOT EXISTS hosts (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  hostname TEXT NOT NULL,
  port INTEGER DEFAULT 22,
  username TEXT NOT NULL,
  auth_method TEXT DEFAULT 'key',
  private_key TEXT,
  status TEXT DEFAULT 'pending',
  last_tested INTEGER,
  created_at INTEGER NOT NULL
);

-- Deploy events (persistent log of all deployment phases, events, build logs)
CREATE TABLE IF NOT EXISTS deploy_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  deployment_id TEXT NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
  event_type TEXT NOT NULL,       -- 'phase', 'log', 'error', 'waiting', 'complete'
  phase TEXT,                     -- current phase when event occurred
  message TEXT,
  data TEXT,                      -- JSON extra data (bridgeAddress, etc.)
  created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_deployments_status ON deployments(status);
CREATE INDEX IF NOT EXISTS idx_hosts_status ON hosts(status);
CREATE INDEX IF NOT EXISTS idx_deploy_events_deployment ON deploy_events(deployment_id, created_at);

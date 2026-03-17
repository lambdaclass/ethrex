-- Users table
CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  email TEXT UNIQUE NOT NULL,
  name TEXT NOT NULL,
  password_hash TEXT,
  auth_provider TEXT DEFAULT 'email',
  role TEXT DEFAULT 'user',
  picture TEXT,
  status TEXT DEFAULT 'active',
  created_at INTEGER NOT NULL
);

-- Guest Programs (Store)
CREATE TABLE IF NOT EXISTS programs (
  id TEXT PRIMARY KEY,
  program_id TEXT UNIQUE NOT NULL,
  program_type_id INTEGER UNIQUE,
  creator_id TEXT NOT NULL REFERENCES users(id),
  name TEXT NOT NULL,
  description TEXT,
  category TEXT DEFAULT 'general',
  icon_url TEXT,
  elf_hash TEXT,
  elf_storage_path TEXT,
  vk_sp1 TEXT,
  vk_risc0 TEXT,
  status TEXT DEFAULT 'pending',
  use_count INTEGER DEFAULT 0,
  batch_count INTEGER DEFAULT 0,
  is_official INTEGER DEFAULT 0,
  created_at INTEGER NOT NULL,
  approved_at INTEGER
);

-- Program usage log
CREATE TABLE IF NOT EXISTS program_usage (
  id TEXT PRIMARY KEY,
  program_id TEXT NOT NULL REFERENCES programs(id),
  user_id TEXT NOT NULL REFERENCES users(id),
  batch_number INTEGER,
  created_at INTEGER NOT NULL
);

-- Program versions (ELF upload history)
CREATE TABLE IF NOT EXISTS program_versions (
  id TEXT PRIMARY KEY,
  program_id TEXT NOT NULL REFERENCES programs(id),
  version INTEGER NOT NULL,
  elf_hash TEXT NOT NULL,
  elf_storage_path TEXT,
  uploaded_by TEXT NOT NULL REFERENCES users(id),
  created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_program_versions_program ON program_versions(program_id);

-- Deployments (user registers their L2 on Platform for Open Appchain)
CREATE TABLE IF NOT EXISTS deployments (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  program_id TEXT NOT NULL REFERENCES programs(id),
  name TEXT NOT NULL,
  chain_id INTEGER,
  rpc_url TEXT,
  status TEXT DEFAULT 'configured',
  config TEXT,
  phase TEXT DEFAULT 'configured',
  bridge_address TEXT,
  proposer_address TEXT,
  description TEXT,
  screenshots TEXT,
  explorer_url TEXT,
  dashboard_url TEXT,
  social_links TEXT,
  l1_chain_id INTEGER,
  network_mode TEXT,
  hashtags TEXT,
  owner_wallet TEXT,
  created_at INTEGER NOT NULL
);

-- Sessions (persistent authentication tokens)
CREATE TABLE IF NOT EXISTS sessions (
  token TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_created ON sessions(created_at);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_programs_status ON programs(status);
CREATE INDEX IF NOT EXISTS idx_programs_category ON programs(category);
CREATE INDEX IF NOT EXISTS idx_programs_creator ON programs(creator_id);
CREATE INDEX IF NOT EXISTS idx_program_usage_program ON program_usage(program_id);
CREATE INDEX IF NOT EXISTS idx_program_usage_user ON program_usage(user_id);
CREATE INDEX IF NOT EXISTS idx_deployments_user ON deployments(user_id);
CREATE INDEX IF NOT EXISTS idx_deployments_program ON deployments(program_id);

-- Social: Reviews (one per wallet per appchain — deployment_id stores either deployment or listing ID)
CREATE TABLE IF NOT EXISTS reviews (
  id TEXT PRIMARY KEY,
  deployment_id TEXT NOT NULL,
  wallet_address TEXT NOT NULL,
  rating INTEGER NOT NULL CHECK(rating >= 1 AND rating <= 5),
  content TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_reviews_deployment ON reviews(deployment_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_reviews_unique ON reviews(deployment_id, wallet_address);

-- Social: Comments
CREATE TABLE IF NOT EXISTS comments (
  id TEXT PRIMARY KEY,
  deployment_id TEXT NOT NULL,
  wallet_address TEXT NOT NULL,
  content TEXT NOT NULL,
  parent_id TEXT REFERENCES comments(id),
  deleted_at INTEGER,
  created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_comments_deployment ON comments(deployment_id);

-- Social: Reactions (likes, one per wallet per target)
CREATE TABLE IF NOT EXISTS reactions (
  id TEXT PRIMARY KEY,
  target_type TEXT NOT NULL CHECK(target_type IN ('review', 'comment')),
  target_id TEXT NOT NULL,
  wallet_address TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_reactions_target ON reactions(target_type, target_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_reactions_unique ON reactions(target_type, target_id, wallet_address);

-- Bookmarks (account-based, one per user per appchain)
CREATE TABLE IF NOT EXISTS bookmarks (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  deployment_id TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_bookmarks_unique ON bookmarks(user_id, deployment_id);
CREATE INDEX IF NOT EXISTS idx_bookmarks_user ON bookmarks(user_id);

-- Announcements (owner-posted, max 10 per appchain)
CREATE TABLE IF NOT EXISTS announcements (
  id TEXT PRIMARY KEY,
  deployment_id TEXT NOT NULL,
  wallet_address TEXT NOT NULL,
  title TEXT NOT NULL,
  content TEXT NOT NULL,
  pinned INTEGER DEFAULT 0,
  created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_announcements_deployment ON announcements(deployment_id);

-- Explore Listings (synced from tokamak-rollup-metadata-repository)
CREATE TABLE IF NOT EXISTS explore_listings (
  id TEXT PRIMARY KEY,
  l1_chain_id INTEGER NOT NULL,
  l2_chain_id INTEGER NOT NULL,
  stack_type TEXT NOT NULL,
  identity_contract TEXT NOT NULL,
  name TEXT NOT NULL,
  rollup_type TEXT,
  status TEXT DEFAULT 'active',
  rpc_url TEXT,
  explorer_url TEXT,
  bridge_url TEXT,
  dashboard_url TEXT,
  native_token_type TEXT DEFAULT 'eth',
  native_token_symbol TEXT DEFAULT 'ETH',
  native_token_decimals INTEGER DEFAULT 18,
  native_token_l1_address TEXT,
  l1_contracts TEXT,
  operator_name TEXT,
  operator_website TEXT,
  operator_social_links TEXT,
  description TEXT,
  screenshots TEXT,
  hashtags TEXT,
  signed_by TEXT,
  signature TEXT,
  owner_wallet TEXT,
  repo_file_path TEXT UNIQUE,
  repo_sha TEXT,
  synced_at INTEGER,
  created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_listings_chain ON explore_listings(l1_chain_id);
CREATE INDEX IF NOT EXISTS idx_listings_stack ON explore_listings(stack_type);
CREATE UNIQUE INDEX IF NOT EXISTS idx_listings_identity ON explore_listings(l1_chain_id, stack_type, identity_contract);

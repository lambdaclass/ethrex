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

-- Deployments (user selects a program for their L2)
CREATE TABLE IF NOT EXISTS deployments (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  program_id TEXT NOT NULL REFERENCES programs(id),
  name TEXT NOT NULL,
  chain_id INTEGER,
  rpc_url TEXT,
  status TEXT DEFAULT 'configured',
  config TEXT,
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

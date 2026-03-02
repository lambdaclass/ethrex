export interface User {
  id: string;
  email: string;
  name: string;
  role: "user" | "admin";
  picture: string | null;
  authProvider?: string;
}

export interface Program {
  id: string;
  program_id: string;
  program_type_id: number | null;
  creator_id: string;
  name: string;
  description: string | null;
  category: string;
  icon_url: string | null;
  elf_hash: string | null;
  elf_storage_path: string | null;
  vk_sp1: string | null;
  vk_risc0: string | null;
  status: "pending" | "active" | "rejected" | "disabled";
  use_count: number;
  batch_count: number;
  is_official: boolean;
  created_at: number;
  approved_at: number | null;
}

export interface Deployment {
  id: string;
  user_id: string;
  program_id: string;
  program_name?: string;
  program_slug?: string;
  category?: string;
  name: string;
  chain_id: number | null;
  rpc_url: string | null;
  status: string;
  config: string | null;
  created_at: number;
}

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
  host_id: string | null;
  docker_project: string | null;
  l1_port: number | null;
  l2_port: number | null;
  phase: string;
  bridge_address: string | null;
  proposer_address: string | null;
  error_message: string | null;
  created_at: number;
}

export interface ContainerStatus {
  Name: string;
  State: string;
  Status: string;
  Service: string;
}

export interface DeploymentStatus {
  phase: string;
  containers: ContainerStatus[];
  endpoints: {
    l1Rpc: string | null;
    l2Rpc: string | null;
  };
  contracts: {
    bridge: string | null;
    proposer: string | null;
  };
  error: string | null;
}

export interface MonitoringData {
  l1: {
    healthy: boolean;
    blockNumber: number | null;
    chainId: number | null;
    balance: string | null;
    rpcUrl: string;
  } | null;
  l2: {
    healthy: boolean;
    blockNumber: number | null;
    chainId: number | null;
    balance: string | null;
    rpcUrl: string;
  } | null;
}

export interface Host {
  id: string;
  user_id: string;
  name: string;
  hostname: string;
  port: number;
  username: string;
  auth_method: string;
  status: string;
  last_tested: number | null;
  created_at: number;
}

export interface DeploymentEvent {
  event: string;
  phase?: string;
  message?: string;
  timestamp: number;
  l1Rpc?: string;
  l2Rpc?: string;
  bridgeAddress?: string;
  proposerAddress?: string;
}

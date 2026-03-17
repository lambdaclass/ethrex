/**
 * EVM wallet authentication for Platform API social features.
 * No external dependencies — uses only browser APIs (MetaMask personal_sign).
 */

interface EthereumProvider {
  request: (args: { method: string; params?: unknown[] }) => Promise<unknown>;
}

function getEthereum(): EthereumProvider | null {
  if (typeof window === "undefined") return null;
  return (window as unknown as { ethereum?: EthereumProvider }).ethereum ?? null;
}

const API_SIGN_MESSAGE =
  "Sign in to Tokamak Appchain Showroom\n\nDomain: platform.tokamak.network\nPurpose: Social interaction authentication\n\nThis signature proves you own this wallet.";

export interface ApiWalletSession {
  address: string;
  signature: string;
}

/** Connect EVM wallet for Platform API auth. */
export async function connectWalletForApi(): Promise<ApiWalletSession> {
  const ethereum = getEthereum();
  if (!ethereum) throw new Error("No wallet found. Please install MetaMask.");

  const accounts = (await ethereum.request({
    method: "eth_requestAccounts",
  })) as unknown;

  if (!Array.isArray(accounts) || accounts.length === 0 || typeof accounts[0] !== "string") {
    throw new Error("Wallet returned no accounts. Please unlock your wallet and try again.");
  }
  const address = accounts[0];

  const signature = (await ethereum.request({
    method: "personal_sign",
    params: [API_SIGN_MESSAGE, address],
  })) as string;

  return { address, signature };
}

/** Check if browser has an EVM wallet available. */
export function hasWallet(): boolean {
  return getEthereum() !== null;
}

/** Shorten an EVM address for display. */
export function shortenAddress(addr: string): string {
  return `${addr.slice(0, 6)}...${addr.slice(-4)}`;
}

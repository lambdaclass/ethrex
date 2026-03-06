import { createPublicClient, http, formatEther } from 'viem';

const client = createPublicClient({
  transport: http('http://localhost:8545'),
});

export async function getBalance(address: string): Promise<string> {
  const balance = await client.getBalance({
    address: address as `0x${string}`,
  });
  return formatEther(balance);
}

export async function getBlockNumber(): Promise<bigint> {
  return client.getBlockNumber();
}

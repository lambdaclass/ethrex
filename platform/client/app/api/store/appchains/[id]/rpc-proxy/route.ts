import { NextRequest, NextResponse } from "next/server";
import { ensureSchema } from "@/lib/db";
import { resolveAppchain } from "@/lib/appchain-resolver";

const ALLOWED_METHODS = [
  "eth_blockNumber", "eth_chainId", "eth_gasPrice",
  "ethrex_batchNumber", "ethrex_metadata", "net_version",
];

export async function POST(
  req: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    await ensureSchema();
    const { id } = await params;
    const appchain = await resolveAppchain(id);
    if (!appchain) {
      return NextResponse.json({ error: "Appchain not found" }, { status: 404 });
    }
    if (!appchain.rpc_url) {
      return NextResponse.json({ error: "No RPC URL configured for this appchain" }, { status: 404 });
    }

    // SSRF protection
    try {
      const rpcUrl = new URL(appchain.rpc_url as string);
      if (!["http:", "https:"].includes(rpcUrl.protocol)) {
        return NextResponse.json({ error: "Invalid RPC URL protocol" }, { status: 400 });
      }
      const host = rpcUrl.hostname;
      if (host === "localhost" || host === "127.0.0.1" || host === "::1" ||
          host.startsWith("10.") || host.startsWith("192.168.") ||
          host.startsWith("169.254.") || host.endsWith(".internal") ||
          /^172\.(1[6-9]|2\d|3[01])\./.test(host)) {
        return NextResponse.json({ error: "RPC URL cannot point to internal addresses" }, { status: 400 });
      }
    } catch {
      return NextResponse.json({ error: "Invalid RPC URL" }, { status: 400 });
    }

    const body = await req.json();
    const { method, params: rpcParams } = body;
    if (!method || !ALLOWED_METHODS.includes(method)) {
      return NextResponse.json({ error: "Method not allowed" }, { status: 400 });
    }

    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 5000);

    const response = await fetch(appchain.rpc_url as string, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params: rpcParams || [] }),
      signal: controller.signal,
    });

    clearTimeout(timeout);
    const data = await response.json();
    return NextResponse.json(data);
  } catch (e) {
    console.error(`[rpc-proxy] Error:`, e);
    return NextResponse.json({ error: "L2 node unreachable" }, { status: 502 });
  }
}

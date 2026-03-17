import { NextRequest, NextResponse } from "next/server";
import { getFileContent } from "@/lib/github-pr";

export async function GET(
  _req: NextRequest,
  { params }: { params: Promise<{ l1ChainId: string; stackType: string; identityAddress: string }> },
) {
  try {
    const { l1ChainId, stackType, identityAddress } = await params;
    const filePath = `tokamak-appchain-data/${l1ChainId}/${stackType}/${identityAddress.toLowerCase()}.json`;
    const existing = await getFileContent(filePath, "main");
    if (existing) {
      return NextResponse.json({
        exists: true,
        createdAt: existing.createdAt || null,
        l2ChainId: existing.l2ChainId || null,
        nativeToken: existing.nativeToken || null,
      });
    }
    return NextResponse.json({ exists: false });
  } catch (e) {
    console.error("[appchain-registry] check failed:", (e as Error).message);
    return NextResponse.json(
      { error: "Failed to check metadata registry" },
      { status: 503 },
    );
  }
}

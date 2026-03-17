import { NextRequest, NextResponse } from "next/server";
import { ensureSchema } from "@/lib/db";
import { resolveAppchain } from "@/lib/appchain-resolver";
import { requireWallet } from "@/lib/wallet-auth";
import { toggleReaction, targetExists } from "@/lib/social-queries";

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

    const walletAddress = requireWallet(req);
    const { targetType, targetId } = await req.json();

    if (!["review", "comment"].includes(targetType)) {
      return NextResponse.json({ error: "targetType must be 'review' or 'comment'" }, { status: 400 });
    }
    if (!targetId || typeof targetId !== "string") {
      return NextResponse.json({ error: "targetId is required" }, { status: 400 });
    }

    if (!(await targetExists(targetType, targetId))) {
      return NextResponse.json({ error: "Target not found" }, { status: 404 });
    }

    const result = await toggleReaction({ targetType, targetId, walletAddress });
    return NextResponse.json(result);
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

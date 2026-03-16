import { NextRequest, NextResponse } from "next/server";
import { ensureSchema } from "@/lib/db";
import { requireWallet } from "@/lib/wallet-auth";
import { deleteComment } from "@/lib/social-queries";

export async function DELETE(
  req: NextRequest,
  { params }: { params: Promise<{ id: string; commentId: string }> }
) {
  try {
    await ensureSchema();
    const { commentId } = await params;
    const walletAddress = requireWallet(req);

    const deleted = await deleteComment(commentId, walletAddress);
    if (!deleted) {
      return NextResponse.json({ error: "Not found" }, { status: 404 });
    }
    return NextResponse.json({ success: true });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

import { NextRequest, NextResponse } from "next/server";
import { ensureSchema } from "@/lib/db";
import { resolveAppchain } from "@/lib/appchain-resolver";
import { requireWallet } from "@/lib/wallet-auth";
import {
  getCommentsByDeployment, createComment,
  getReactionCounts, getUserReactions,
} from "@/lib/social-queries";

export async function GET(
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

    const comments = await getCommentsByDeployment(id);
    const commentIds = comments.map((c) => c.id as string);
    const reactionCounts = await getReactionCounts("comment", commentIds);

    const walletAddress = req.headers.get("x-wallet-address");
    const userReactions = walletAddress
      ? await getUserReactions("comment", commentIds, walletAddress)
      : [];

    return NextResponse.json({ comments, reactionCounts, userReactions });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

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
    const { content, parentId } = await req.json();

    if (!content || typeof content !== "string" || content.trim().length === 0) {
      return NextResponse.json({ error: "Content is required" }, { status: 400 });
    }
    if (content.length > 500) {
      return NextResponse.json({ error: "Content must be 500 characters or less" }, { status: 400 });
    }

    const comment = await createComment({
      deploymentId: id,
      walletAddress,
      content: content.trim(),
      parentId: parentId || null,
    });

    return NextResponse.json({ comment }, { status: 201 });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

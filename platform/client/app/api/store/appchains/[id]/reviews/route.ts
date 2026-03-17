import { NextRequest, NextResponse } from "next/server";
import { ensureSchema } from "@/lib/db";
import { resolveAppchain } from "@/lib/appchain-resolver";
import { requireWallet } from "@/lib/wallet-auth";
import {
  getReviewsByDeployment, createReview,
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

    const reviews = await getReviewsByDeployment(id);
    const reviewIds = reviews.map((r) => r.id as string);
    const reactionCounts = await getReactionCounts("review", reviewIds);

    const walletAddress = req.headers.get("x-wallet-address");
    const userReactions = walletAddress
      ? await getUserReactions("review", reviewIds, walletAddress)
      : [];

    return NextResponse.json({ reviews, reactionCounts, userReactions });
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
    const { rating, content } = await req.json();

    if (!Number.isInteger(rating) || rating < 1 || rating > 5) {
      return NextResponse.json({ error: "Rating must be an integer 1-5" }, { status: 400 });
    }
    if (!content || typeof content !== "string" || content.trim().length === 0) {
      return NextResponse.json({ error: "Content is required" }, { status: 400 });
    }
    if (content.length > 500) {
      return NextResponse.json({ error: "Content must be 500 characters or less" }, { status: 400 });
    }

    const review = await createReview({
      deploymentId: id,
      walletAddress,
      rating,
      content: content.trim(),
    });

    return NextResponse.json({ review }, { status: 201 });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

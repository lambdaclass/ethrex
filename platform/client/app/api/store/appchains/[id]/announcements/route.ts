import { NextRequest, NextResponse } from "next/server";
import { ensureSchema } from "@/lib/db";
import { resolveAppchain } from "@/lib/appchain-resolver";
import { requireWallet } from "@/lib/wallet-auth";
import { getAnnouncements, createAnnouncement, getAnnouncementCount } from "@/lib/social-queries";

export async function GET(
  _req: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    await ensureSchema();
    const { id } = await params;
    const appchain = await resolveAppchain(id);
    if (!appchain) {
      return NextResponse.json({ error: "Appchain not found" }, { status: 404 });
    }

    const announcements = await getAnnouncements(id);
    return NextResponse.json({ announcements });
  } catch (e) {
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

    // Owner check
    const ownerWallet = ((appchain.owner_wallet || appchain.signed_by || "") as string).toLowerCase();
    if (!ownerWallet || walletAddress !== ownerWallet) {
      return NextResponse.json({ error: "Only the appchain owner can perform this action" }, { status: 403 });
    }

    const { title, content, pinned } = await req.json();
    if (!title || typeof title !== "string" || title.trim().length === 0) {
      return NextResponse.json({ error: "Title is required" }, { status: 400 });
    }
    if (title.length > 100) {
      return NextResponse.json({ error: "Title must be 100 characters or less" }, { status: 400 });
    }
    if (!content || typeof content !== "string" || content.trim().length === 0) {
      return NextResponse.json({ error: "Content is required" }, { status: 400 });
    }
    if (content.length > 2000) {
      return NextResponse.json({ error: "Content must be 2000 characters or less" }, { status: 400 });
    }

    const count = await getAnnouncementCount(id);
    if (count >= 10) {
      return NextResponse.json({ error: "Maximum 10 announcements per appchain" }, { status: 400 });
    }

    const announcement = await createAnnouncement({
      deploymentId: id,
      walletAddress,
      title: title.trim(),
      content: content.trim(),
      pinned: !!pinned,
    });

    return NextResponse.json({ announcement }, { status: 201 });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

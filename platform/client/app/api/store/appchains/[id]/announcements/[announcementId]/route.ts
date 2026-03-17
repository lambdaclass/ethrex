import { NextRequest, NextResponse } from "next/server";
import { ensureSchema } from "@/lib/db";
import { resolveAppchain } from "@/lib/appchain-resolver";
import { requireWallet } from "@/lib/wallet-auth";
import { updateAnnouncement, deleteAnnouncement } from "@/lib/social-queries";

async function resolveAndCheckOwner(req: NextRequest, id: string) {
  await ensureSchema();
  const appchain = await resolveAppchain(id);
  if (!appchain) {
    throw NextResponse.json({ error: "Appchain not found" }, { status: 404 });
  }
  const walletAddress = requireWallet(req);
  const ownerWallet = ((appchain.owner_wallet || appchain.signed_by || "") as string).toLowerCase();
  if (!ownerWallet || walletAddress !== ownerWallet) {
    throw NextResponse.json({ error: "Only the appchain owner can perform this action" }, { status: 403 });
  }
  return walletAddress;
}

export async function PUT(
  req: NextRequest,
  { params }: { params: Promise<{ id: string; announcementId: string }> }
) {
  try {
    const { id, announcementId } = await params;
    const walletAddress = await resolveAndCheckOwner(req, id);

    const { title, content, pinned } = await req.json();
    if (!title || typeof title !== "string" || title.trim().length === 0) {
      return NextResponse.json({ error: "Title is required" }, { status: 400 });
    }
    if (!content || typeof content !== "string" || content.trim().length === 0) {
      return NextResponse.json({ error: "Content is required" }, { status: 400 });
    }

    const updated = await updateAnnouncement(announcementId, walletAddress, {
      title: title.trim(),
      content: content.trim(),
      pinned: !!pinned,
    });
    if (!updated) {
      return NextResponse.json({ error: "Announcement not found" }, { status: 404 });
    }
    return NextResponse.json({ announcement: updated });
  } catch (e) {
    if (e instanceof Response) return e;
    if (e instanceof NextResponse) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

export async function DELETE(
  req: NextRequest,
  { params }: { params: Promise<{ id: string; announcementId: string }> }
) {
  try {
    const { id, announcementId } = await params;
    const walletAddress = await resolveAndCheckOwner(req, id);

    const deleted = await deleteAnnouncement(announcementId, walletAddress);
    if (!deleted) {
      return NextResponse.json({ error: "Announcement not found" }, { status: 404 });
    }
    return NextResponse.json({ success: true });
  } catch (e) {
    if (e instanceof Response) return e;
    if (e instanceof NextResponse) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

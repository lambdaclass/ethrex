import { NextRequest, NextResponse } from "next/server";
import { ensureSchema } from "@/lib/db";
import { requireAuth } from "@/lib/auth";
import { getUserBookmarks } from "@/lib/social-queries";

export async function GET(req: NextRequest) {
  try {
    await ensureSchema();
    const user = await requireAuth(req);
    const bookmarks = await getUserBookmarks(user.id);
    return NextResponse.json({ bookmarks });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

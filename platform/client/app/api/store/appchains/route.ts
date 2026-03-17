import { NextRequest, NextResponse } from "next/server";
import { ensureSchema } from "@/lib/db";
import { getListings, getActiveDeployments } from "@/lib/appchain-resolver";
import { getSocialStatsBatch } from "@/lib/social-queries";

export async function GET(req: NextRequest) {
  try {
    await ensureSchema();
    const { searchParams } = req.nextUrl;
    const search = searchParams.get("search");
    const stackType = searchParams.get("stack_type");
    const l1ChainId = searchParams.get("l1_chain_id");
    const limit = Math.max(1, parseInt(searchParams.get("limit") || "50", 10));
    const offset = Math.max(0, parseInt(searchParams.get("offset") || "0", 10));

    // Fetch from both sources (no offset — pagination applied after merge)
    const fetchLimit = limit + offset;
    const listings = await getListings({
      search, stackType, l1ChainId,
      limit: fetchLimit, offset: 0,
    });
    const deployments = await getActiveDeployments({
      search, limit: fetchLimit, offset: 0,
    });

    // Merge: listings first, then deployments (deduplicate by ID)
    const seenIds = new Set<string>();
    const merged: Record<string, unknown>[] = [];
    for (const item of [...listings, ...deployments]) {
      const id = item.id as string;
      if (!seenIds.has(id)) {
        seenIds.add(id);
        merged.push(item);
      }
    }

    // Apply pagination after merge
    const paged = merged.slice(offset, offset + limit);

    // Enrich with social stats
    const ids = paged.map((a) => a.id as string);
    const stats = await getSocialStatsBatch(ids);
    const enriched = paged.map((a) => {
      let hashtags: string[] = [];
      try { hashtags = a.hashtags ? JSON.parse(a.hashtags as string) : []; } catch { /* ignore */ }
      const s = stats[a.id as string];
      return {
        ...a,
        hashtags,
        avg_rating: s?.avg_rating ?? null,
        review_count: s?.review_count ?? 0,
        comment_count: s?.comment_count ?? 0,
      };
    });

    return NextResponse.json({ appchains: enriched });
  } catch (e) {
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

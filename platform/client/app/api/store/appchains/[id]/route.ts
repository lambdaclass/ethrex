import { NextRequest, NextResponse } from "next/server";
import { ensureSchema } from "@/lib/db";
import { resolveAppchain } from "@/lib/appchain-resolver";
import { getSocialStats } from "@/lib/social-queries";

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

    let screenshots: string[] = [];
    let social_links: Record<string, string> = {};
    let l1_contracts: Record<string, string> = {};
    try { screenshots = appchain.screenshots ? JSON.parse(appchain.screenshots as string) : []; } catch { /* ignore */ }
    try {
      social_links = appchain.social_links ? JSON.parse(appchain.social_links as string) : {};
      if (!social_links || typeof social_links !== "object") social_links = {};
      if (appchain.operator_social_links && Object.keys(social_links).length === 0) {
        social_links = JSON.parse(appchain.operator_social_links as string);
      }
    } catch { /* ignore */ }
    try { l1_contracts = appchain.l1_contracts ? JSON.parse(appchain.l1_contracts as string) : {}; } catch { /* ignore */ }

    const stats = await getSocialStats(id);

    return NextResponse.json({
      appchain: { ...appchain, screenshots, social_links, l1_contracts, ...stats },
    });
  } catch (e) {
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

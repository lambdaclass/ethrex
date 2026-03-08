import { NextRequest, NextResponse } from "next/server";
import { requireAuth } from "@/lib/auth";
import { getUsage } from "@/lib/token-limiter";

export async function GET(req: NextRequest) {
  let user;
  try {
    user = await requireAuth(req);
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: "auth_error" }, { status: 401 });
  }

  const dailyLimit = user.daily_ai_limit || 50000;
  const usage = await getUsage(user.id, dailyLimit);
  return NextResponse.json(usage);
}

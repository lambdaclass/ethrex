import { NextRequest, NextResponse } from "next/server";
import { getSessionUser, destroySession } from "@/lib/auth";

export async function POST(req: NextRequest) {
  try {
    const result = await getSessionUser(req);
    if (result) {
      await destroySession(result.token);
    }
    return NextResponse.json({ ok: true });
  } catch (e) {
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

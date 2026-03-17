import { NextRequest, NextResponse } from "next/server";
import { createSession } from "@/lib/auth";
import { exchangeNaverCode, getNaverClientId } from "@/lib/oauth";
import { findOrCreateOAuthUser } from "@/lib/oauth-user";

export async function GET() {
  const clientId = getNaverClientId();
  if (!clientId) return NextResponse.json({ error: "Naver auth not configured" }, { status: 404 });
  return NextResponse.json({ clientId });
}

export async function POST(req: NextRequest) {
  try {
    const { code, state } = await req.json();
    if (!code) return NextResponse.json({ error: "code is required" }, { status: 400 });

    const profile = await exchangeNaverCode(code, state);
    const user = await findOrCreateOAuthUser(profile, "naver");
    const token = await createSession(user.id);

    return NextResponse.json({
      token,
      user: { id: user.id, email: user.email, name: user.name, role: user.role, picture: user.picture },
    });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 401 });
  }
}

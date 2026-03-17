import { NextRequest, NextResponse } from "next/server";
import { createSession } from "@/lib/auth";
import { exchangeKakaoCode, getKakaoClientId } from "@/lib/oauth";
import { findOrCreateOAuthUser } from "@/lib/oauth-user";

export async function GET() {
  const clientId = getKakaoClientId();
  if (!clientId) return NextResponse.json({ error: "Kakao auth not configured" }, { status: 404 });
  return NextResponse.json({ clientId });
}

export async function POST(req: NextRequest) {
  try {
    const { code, redirectUri } = await req.json();
    if (!code) return NextResponse.json({ error: "code is required" }, { status: 400 });

    const profile = await exchangeKakaoCode(code, redirectUri);
    const user = await findOrCreateOAuthUser(profile, "kakao");
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

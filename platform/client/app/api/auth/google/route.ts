import { NextRequest, NextResponse } from "next/server";
import { createSession } from "@/lib/auth";
import { verifyGoogleIdToken, getGoogleClientId } from "@/lib/oauth";
import { findOrCreateOAuthUser } from "@/lib/oauth-user";

export async function GET() {
  const clientId = getGoogleClientId();
  if (!clientId) return NextResponse.json({ error: "Google auth not configured" }, { status: 404 });
  return NextResponse.json({ clientId });
}

export async function POST(req: NextRequest) {
  try {
    const { idToken } = await req.json();
    if (!idToken) return NextResponse.json({ error: "idToken is required" }, { status: 400 });

    const profile = await verifyGoogleIdToken(idToken);
    const user = await findOrCreateOAuthUser(profile, "google");
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

/**
 * Desktop token retrieval API with PKCE verification.
 * GET: Poll for session token — requires code_verifier to prove ownership.
 */
import { NextRequest, NextResponse } from "next/server";
import { sql } from "@/lib/db";
import { ensureDesktopTable, verifyCodeChallenge, CODE_TTL_MS } from "@/lib/desktop-auth";

export async function GET(req: NextRequest) {
  await ensureDesktopTable();

  const code = req.nextUrl.searchParams.get("code");
  const codeVerifier = req.nextUrl.searchParams.get("code_verifier");

  if (!code || !codeVerifier) {
    return NextResponse.json({ error: "code_and_verifier_required" }, { status: 400 });
  }

  const { rows } = await sql`SELECT session_token, code_challenge, created_at FROM desktop_auth_codes WHERE code = ${code}`;
  if (rows.length === 0) {
    return NextResponse.json({ error: "invalid_code" }, { status: 404 });
  }

  const row = rows[0];

  // Check expiry
  if (Date.now() - Number(row.created_at) > CODE_TTL_MS) {
    await sql`DELETE FROM desktop_auth_codes WHERE code = ${code}`;
    return NextResponse.json({ error: "code_expired" }, { status: 410 });
  }

  // Verify PKCE: SHA-256(code_verifier) must match stored code_challenge
  const valid = await verifyCodeChallenge(codeVerifier, row.code_challenge as string);
  if (!valid) {
    return NextResponse.json({ error: "invalid_verifier" }, { status: 403 });
  }

  // Not yet linked to a session
  if (!row.session_token) {
    return NextResponse.json({ status: "pending" });
  }

  // Token is ready — delete code and return token
  await sql`DELETE FROM desktop_auth_codes WHERE code = ${code}`;
  return NextResponse.json({ status: "ready", token: row.session_token });
}

/**
 * Desktop auth code API with PKCE.
 * POST: Generate a temporary code (requires code_challenge from desktop app).
 * PUT: Link a session token to a code after successful login (requires valid session).
 */
import { NextRequest, NextResponse } from "next/server";
import { sql } from "@/lib/db";
import { getSessionUser } from "@/lib/auth";
import { ensureDesktopTable, CODE_TTL_MS } from "@/lib/desktop-auth";

export async function POST(req: NextRequest) {
  await ensureDesktopTable();

  let body: { code_challenge?: string };
  try {
    body = await req.json();
  } catch {
    return NextResponse.json({ error: "invalid_json" }, { status: 400 });
  }

  if (!body.code_challenge) {
    return NextResponse.json({ error: "code_challenge_required" }, { status: 400 });
  }

  const code = "dc_" + Array.from(crypto.getRandomValues(new Uint8Array(16)))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");

  // Clean up expired codes
  await sql`DELETE FROM desktop_auth_codes WHERE created_at < ${Date.now() - CODE_TTL_MS}`;

  await sql`INSERT INTO desktop_auth_codes (code, code_challenge, created_at) VALUES (${code}, ${body.code_challenge}, ${Date.now()})`;

  return NextResponse.json({ code, expires_in: CODE_TTL_MS / 1000 });
}

export async function PUT(req: NextRequest) {
  await ensureDesktopTable();

  // Verify the caller is authenticated
  const session = await getSessionUser(req);
  if (!session) {
    return NextResponse.json({ error: "auth_required" }, { status: 401 });
  }

  let body: { code?: string };
  try {
    body = await req.json();
  } catch {
    return NextResponse.json({ error: "invalid_json" }, { status: 400 });
  }

  if (!body.code) {
    return NextResponse.json({ error: "code_required" }, { status: 400 });
  }

  const { rows } = await sql`SELECT created_at FROM desktop_auth_codes WHERE code = ${body.code}`;
  if (rows.length === 0) {
    return NextResponse.json({ error: "invalid_code" }, { status: 404 });
  }

  if (Date.now() - Number(rows[0].created_at) > CODE_TTL_MS) {
    await sql`DELETE FROM desktop_auth_codes WHERE code = ${body.code}`;
    return NextResponse.json({ error: "code_expired" }, { status: 410 });
  }

  // Link the authenticated user's session token to the desktop code
  await sql`UPDATE desktop_auth_codes SET session_token = ${session.token} WHERE code = ${body.code}`;

  return NextResponse.json({ ok: true });
}

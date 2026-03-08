/**
 * Desktop token retrieval API.
 * GET: Poll for session token after desktop login flow completes.
 */
import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";

const CODE_TTL_MS = 5 * 60 * 1000; // 5 minutes

export async function GET(req: NextRequest) {
  await ensureSchema();

  const code = req.nextUrl.searchParams.get("code");
  if (!code) {
    return NextResponse.json({ error: "code_required" }, { status: 400 });
  }

  const { rows } = await sql`SELECT session_token, created_at FROM desktop_auth_codes WHERE code = ${code}`;
  if (rows.length === 0) {
    return NextResponse.json({ error: "invalid_code" }, { status: 404 });
  }

  const row = rows[0];

  // Check expiry
  if (Date.now() - Number(row.created_at) > CODE_TTL_MS) {
    await sql`DELETE FROM desktop_auth_codes WHERE code = ${code}`;
    return NextResponse.json({ error: "code_expired" }, { status: 410 });
  }

  // Not yet linked to a session
  if (!row.session_token) {
    return NextResponse.json({ status: "pending" });
  }

  // Token is ready - delete code and return token
  await sql`DELETE FROM desktop_auth_codes WHERE code = ${code}`;
  return NextResponse.json({ status: "ready", token: row.session_token });
}

/**
 * Desktop auth code API.
 * POST: Generate a temporary code for desktop app login flow.
 * PUT: Link a session token to a code after successful login.
 */
import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";

const CODE_TTL_MS = 5 * 60 * 1000; // 5 minutes

async function ensureDesktopTable() {
  await ensureSchema();
  await sql`
    CREATE TABLE IF NOT EXISTS desktop_auth_codes (
      code TEXT PRIMARY KEY,
      session_token TEXT,
      created_at BIGINT NOT NULL
    )
  `;
}

export async function POST() {
  await ensureDesktopTable();

  const code = "dc_" + Array.from(crypto.getRandomValues(new Uint8Array(16)))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");

  // Clean up expired codes
  await sql`DELETE FROM desktop_auth_codes WHERE created_at < ${Date.now() - CODE_TTL_MS}`;

  await sql`INSERT INTO desktop_auth_codes (code, created_at) VALUES (${code}, ${Date.now()})`;

  return NextResponse.json({ code, expires_in: CODE_TTL_MS / 1000 });
}

export async function PUT(req: NextRequest) {
  await ensureDesktopTable();

  let body: { code?: string; token?: string };
  try {
    body = await req.json();
  } catch {
    return NextResponse.json({ error: "invalid_json" }, { status: 400 });
  }

  if (!body.code || !body.token) {
    return NextResponse.json({ error: "code_and_token_required" }, { status: 400 });
  }

  const { rows } = await sql`SELECT created_at FROM desktop_auth_codes WHERE code = ${body.code}`;
  if (rows.length === 0) {
    return NextResponse.json({ error: "invalid_code" }, { status: 404 });
  }

  if (Date.now() - Number(rows[0].created_at) > CODE_TTL_MS) {
    await sql`DELETE FROM desktop_auth_codes WHERE code = ${body.code}`;
    return NextResponse.json({ error: "code_expired" }, { status: 410 });
  }

  await sql`UPDATE desktop_auth_codes SET session_token = ${body.token} WHERE code = ${body.code}`;

  return NextResponse.json({ ok: true });
}

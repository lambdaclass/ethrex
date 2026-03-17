import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAdmin } from "@/lib/auth";

export async function GET(req: NextRequest) {
  try {
    await requireAdmin(req);
    await ensureSchema();

    const { rows } = await sql`
      SELECT id, email, name, role, auth_provider, status, created_at
      FROM users ORDER BY created_at DESC
    `;
    return NextResponse.json({ users: rows });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

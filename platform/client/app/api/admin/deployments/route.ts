import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAdmin } from "@/lib/auth";

export async function GET(req: NextRequest) {
  try {
    await requireAdmin(req);
    await ensureSchema();

    const { rows } = await sql`
      SELECT d.*, p.name as program_name, p.program_id as program_slug,
             u.name as user_name, u.email as user_email
      FROM deployments d
      JOIN programs p ON d.program_id = p.id
      JOIN users u ON d.user_id = u.id
      ORDER BY d.created_at DESC
    `;
    return NextResponse.json({ deployments: rows });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

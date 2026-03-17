import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAdmin } from "@/lib/auth";

export async function GET(req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  try {
    await requireAdmin(req);
    await ensureSchema();
    const { id } = await params;

    const { rows } = await sql`SELECT * FROM programs WHERE id = ${id}`;
    if (rows.length === 0) {
      return NextResponse.json({ error: "Program not found" }, { status: 404 });
    }

    const { rows: creatorRows } = await sql`
      SELECT id, email, name, role FROM users WHERE id = ${rows[0].creator_id}
    `;

    return NextResponse.json({
      program: rows[0],
      creator: creatorRows[0] || null,
    });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAuth } from "@/lib/auth";

export async function POST(req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  try {
    const user = await requireAuth(req);
    await ensureSchema();
    const { id } = await params;
    const { rows } = await sql`SELECT * FROM deployments WHERE id = ${id} AND user_id = ${user.id}`;
    if (rows.length === 0) {
      return NextResponse.json({ error: "Deployment not found" }, { status: 404 });
    }

    await sql`UPDATE deployments SET status = 'active' WHERE id = ${id}`;
    const { rows: updated } = await sql`
      SELECT d.*, p.name as program_name, p.program_id as program_slug, p.category
      FROM deployments d JOIN programs p ON d.program_id = p.id WHERE d.id = ${id}
    `;
    return NextResponse.json({ deployment: updated[0] });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

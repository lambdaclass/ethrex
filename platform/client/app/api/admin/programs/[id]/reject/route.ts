import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAdmin } from "@/lib/auth";

export async function PUT(req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  try {
    await requireAdmin(req);
    await ensureSchema();
    const { id } = await params;

    const { rows } = await sql`SELECT * FROM programs WHERE id = ${id}`;
    if (rows.length === 0) {
      return NextResponse.json({ error: "Program not found" }, { status: 404 });
    }
    if (rows[0].status !== "pending") {
      return NextResponse.json({ error: `Cannot reject program with status '${rows[0].status}'` }, { status: 400 });
    }

    const { rows: updated } = await sql`UPDATE programs SET status = 'rejected' WHERE id = ${id} RETURNING *`;
    return NextResponse.json({ program: updated[0] });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

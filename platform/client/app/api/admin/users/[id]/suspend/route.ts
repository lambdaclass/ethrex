import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAdmin } from "@/lib/auth";

export async function PUT(req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  try {
    const admin = await requireAdmin(req);
    await ensureSchema();
    const { id } = await params;

    if (admin.id === id) {
      return NextResponse.json({ error: "Cannot suspend yourself" }, { status: 400 });
    }
    if (id === "system") {
      return NextResponse.json({ error: "Cannot suspend the system user" }, { status: 400 });
    }

    const { rows } = await sql`SELECT * FROM users WHERE id = ${id}`;
    if (rows.length === 0) {
      return NextResponse.json({ error: "User not found" }, { status: 404 });
    }

    const { rows: updated } = await sql`
      UPDATE users SET status = 'suspended' WHERE id = ${id}
      RETURNING id, email, name, role, auth_provider, status, created_at
    `;
    return NextResponse.json({ user: updated[0] });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAdmin } from "@/lib/auth";

export async function PUT(req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  try {
    await requireAdmin(req);
    await ensureSchema();
    const { id } = await params;
    const { role } = await req.json();

    if (!role || !["user", "admin"].includes(role)) {
      return NextResponse.json({ error: "role must be 'user' or 'admin'" }, { status: 400 });
    }

    const { rows } = await sql`SELECT * FROM users WHERE id = ${id}`;
    if (rows.length === 0) {
      return NextResponse.json({ error: "User not found" }, { status: 404 });
    }

    await sql`UPDATE users SET role = ${role} WHERE id = ${id}`;
    const { rows: updated } = await sql`
      SELECT id, email, name, role, auth_provider, status, created_at FROM users WHERE id = ${id}
    `;
    return NextResponse.json({ user: updated[0] });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

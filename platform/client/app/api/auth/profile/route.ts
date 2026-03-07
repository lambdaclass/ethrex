import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAuth } from "@/lib/auth";
import { isValidName } from "@/lib/validate";

export async function PUT(req: NextRequest) {
  try {
    const user = await requireAuth(req);
    await ensureSchema();
    const { name } = await req.json();

    if (!isValidName(name)) {
      return NextResponse.json({ error: "Name must be 1-100 characters" }, { status: 400 });
    }

    await sql`UPDATE users SET name = ${name.trim()} WHERE id = ${user.id}`;
    const { rows } = await sql`SELECT id, email, name, role, picture, auth_provider FROM users WHERE id = ${user.id}`;
    return NextResponse.json({
      user: { ...rows[0], authProvider: rows[0].auth_provider },
    });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

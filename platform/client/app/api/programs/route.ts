import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAuth } from "@/lib/auth";
import { isValidProgramId, isValidName, isValidCategory } from "@/lib/validate";

export async function POST(req: NextRequest) {
  try {
    const user = await requireAuth(req);
    await ensureSchema();
    const { programId, name, description, category } = await req.json();

    if (!programId || !name) {
      return NextResponse.json({ error: "programId and name are required" }, { status: 400 });
    }
    if (!isValidProgramId(programId)) {
      return NextResponse.json({ error: "programId must be 3-64 lowercase letters, numbers, and hyphens" }, { status: 400 });
    }
    if (!isValidName(name)) {
      return NextResponse.json({ error: "Name must be 1-100 characters" }, { status: 400 });
    }
    if (category && !isValidCategory(category)) {
      return NextResponse.json({ error: "Invalid category" }, { status: 400 });
    }

    const { rows: existing } = await sql`SELECT 1 FROM programs WHERE program_id = ${programId}`;
    if (existing.length > 0) {
      return NextResponse.json({ error: "programId already exists" }, { status: 409 });
    }

    const id = crypto.randomUUID();
    const now = Date.now();
    await sql`
      INSERT INTO programs (id, program_id, creator_id, name, description, category, created_at)
      VALUES (${id}, ${programId}, ${user.id}, ${name.trim()}, ${description || null}, ${category || "general"}, ${now})
    `;

    const { rows } = await sql`SELECT * FROM programs WHERE id = ${id}`;
    return NextResponse.json({ program: rows[0] }, { status: 201 });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

export async function GET(req: NextRequest) {
  try {
    const user = await requireAuth(req);
    await ensureSchema();
    const { rows } = await sql`
      SELECT * FROM programs WHERE creator_id = ${user.id} ORDER BY created_at DESC
    `;
    return NextResponse.json({ programs: rows });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

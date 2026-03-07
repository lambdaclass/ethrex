import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAuth } from "@/lib/auth";
import { isValidName, isValidCategory, sanitizeString } from "@/lib/validate";

export async function GET(req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  try {
    const user = await requireAuth(req);
    await ensureSchema();
    const { id } = await params;
    const { rows } = await sql`SELECT * FROM programs WHERE id = ${id} AND creator_id = ${user.id}`;
    if (rows.length === 0) {
      return NextResponse.json({ error: "Program not found" }, { status: 404 });
    }
    return NextResponse.json({ program: rows[0] });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

export async function PUT(req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  try {
    const user = await requireAuth(req);
    await ensureSchema();
    const { id } = await params;
    const { rows } = await sql`SELECT * FROM programs WHERE id = ${id} AND creator_id = ${user.id}`;
    if (rows.length === 0) {
      return NextResponse.json({ error: "Program not found" }, { status: 404 });
    }

    const body = await req.json();
    if (body.name !== undefined) {
      if (!isValidName(body.name)) {
        return NextResponse.json({ error: "Name must be 1-100 characters" }, { status: 400 });
      }
      await sql`UPDATE programs SET name = ${body.name.trim()} WHERE id = ${id}`;
    }
    if (body.description !== undefined) {
      const desc = sanitizeString(body.description, 2000);
      await sql`UPDATE programs SET description = ${desc} WHERE id = ${id}`;
    }
    if (body.category !== undefined) {
      if (!isValidCategory(body.category)) {
        return NextResponse.json({ error: "Invalid category" }, { status: 400 });
      }
      await sql`UPDATE programs SET category = ${body.category} WHERE id = ${id}`;
    }

    const { rows: updated } = await sql`SELECT * FROM programs WHERE id = ${id}`;
    return NextResponse.json({ program: updated[0] });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

export async function DELETE(req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  try {
    const user = await requireAuth(req);
    await ensureSchema();
    const { id } = await params;
    const { rows } = await sql`SELECT * FROM programs WHERE id = ${id} AND creator_id = ${user.id}`;
    if (rows.length === 0) {
      return NextResponse.json({ error: "Program not found" }, { status: 404 });
    }

    await sql`UPDATE programs SET status = 'disabled' WHERE id = ${id}`;
    const { rows: updated } = await sql`SELECT * FROM programs WHERE id = ${id}`;
    return NextResponse.json({ program: updated[0] });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

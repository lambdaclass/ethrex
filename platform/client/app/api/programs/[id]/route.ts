import { NextRequest, NextResponse } from "next/server";
import { sql, sqlUpdate, ensureSchema } from "@/lib/db";
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
    const fields: Record<string, unknown> = {};

    if (body.name !== undefined) {
      if (!isValidName(body.name)) {
        return NextResponse.json({ error: "Name must be 1-100 characters" }, { status: 400 });
      }
      fields.name = body.name.trim();
    }
    if (body.description !== undefined) {
      fields.description = sanitizeString(body.description, 2000);
    }
    if (body.category !== undefined) {
      if (!isValidCategory(body.category)) {
        return NextResponse.json({ error: "Invalid category" }, { status: 400 });
      }
      fields.category = body.category;
    }

    if (Object.keys(fields).length > 0) {
      await sqlUpdate("programs", id, fields);
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

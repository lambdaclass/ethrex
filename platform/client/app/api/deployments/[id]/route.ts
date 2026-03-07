import { NextRequest, NextResponse } from "next/server";
import { sql, sqlUpdate, ensureSchema } from "@/lib/db";
import { requireAuth } from "@/lib/auth";

export async function GET(req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  try {
    const user = await requireAuth(req);
    await ensureSchema();
    const { id } = await params;
    const { rows } = await sql`
      SELECT d.*, p.name as program_name, p.program_id as program_slug, p.category
      FROM deployments d JOIN programs p ON d.program_id = p.id
      WHERE d.id = ${id} AND d.user_id = ${user.id}
    `;
    if (rows.length === 0) {
      return NextResponse.json({ error: "Deployment not found" }, { status: 404 });
    }
    return NextResponse.json({ deployment: rows[0] });
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
    const { rows } = await sql`SELECT * FROM deployments WHERE id = ${id} AND user_id = ${user.id}`;
    if (rows.length === 0) {
      return NextResponse.json({ error: "Deployment not found" }, { status: 404 });
    }

    const body = await req.json();
    const allowed = ["name", "chain_id", "rpc_url", "status", "config", "phase", "bridge_address", "proposer_address"];
    const fields: Record<string, unknown> = {};
    for (const key of allowed) {
      if (body[key] !== undefined) {
        fields[key] = key === "config" && typeof body[key] === "object" ? JSON.stringify(body[key]) : body[key];
      }
    }

    if (Object.keys(fields).length > 0) {
      await sqlUpdate("deployments", id, fields);
    }

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

export async function DELETE(req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  try {
    const user = await requireAuth(req);
    await ensureSchema();
    const { id } = await params;
    const { rows } = await sql`SELECT * FROM deployments WHERE id = ${id} AND user_id = ${user.id}`;
    if (rows.length === 0) {
      return NextResponse.json({ error: "Deployment not found" }, { status: 404 });
    }
    await sql`DELETE FROM deployments WHERE id = ${id}`;
    return NextResponse.json({ ok: true });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

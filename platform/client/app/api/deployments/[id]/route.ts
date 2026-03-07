import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
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
    const configJson = body.config && typeof body.config === "object" ? JSON.stringify(body.config) : body.config;

    // Update each allowed field individually with safe parameterized queries
    if (body.name !== undefined) await sql`UPDATE deployments SET name = ${body.name} WHERE id = ${id}`;
    if (body.chain_id !== undefined) await sql`UPDATE deployments SET chain_id = ${body.chain_id} WHERE id = ${id}`;
    if (body.rpc_url !== undefined) await sql`UPDATE deployments SET rpc_url = ${body.rpc_url} WHERE id = ${id}`;
    if (body.status !== undefined) await sql`UPDATE deployments SET status = ${body.status} WHERE id = ${id}`;
    if (body.config !== undefined) await sql`UPDATE deployments SET config = ${configJson} WHERE id = ${id}`;
    if (body.phase !== undefined) await sql`UPDATE deployments SET phase = ${body.phase} WHERE id = ${id}`;
    if (body.bridge_address !== undefined) await sql`UPDATE deployments SET bridge_address = ${body.bridge_address} WHERE id = ${id}`;
    if (body.proposer_address !== undefined) await sql`UPDATE deployments SET proposer_address = ${body.proposer_address} WHERE id = ${id}`;

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

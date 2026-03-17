import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAuth } from "@/lib/auth";

export async function POST(req: NextRequest) {
  try {
    const user = await requireAuth(req);
    await ensureSchema();
    const { programId, name, chainId, rpcUrl, config } = await req.json();

    if (!programId || !name) {
      return NextResponse.json({ error: "programId and name are required" }, { status: 400 });
    }

    const { rows: programRows } = await sql`SELECT * FROM programs WHERE id = ${programId} AND status = 'active'`;
    if (programRows.length === 0) {
      return NextResponse.json({ error: "Program not found or not active" }, { status: 404 });
    }

    const id = crypto.randomUUID();
    const now = Date.now();
    const configJson = config ? JSON.stringify(config) : null;
    await sql`
      INSERT INTO deployments (id, user_id, program_id, name, chain_id, rpc_url, config, created_at)
      VALUES (${id}, ${user.id}, ${programId}, ${name.trim()}, ${chainId || null}, ${rpcUrl || null}, ${configJson}, ${now})
    `;

    await sql`UPDATE programs SET use_count = use_count + 1 WHERE id = ${programId}`;

    const { rows } = await sql`
      SELECT d.*, p.name as program_name, p.program_id as program_slug, p.category
      FROM deployments d JOIN programs p ON d.program_id = p.id WHERE d.id = ${id}
    `;
    return NextResponse.json({ deployment: rows[0] }, { status: 201 });
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
      SELECT d.*, p.name as program_name, p.program_id as program_slug, p.category
      FROM deployments d JOIN programs p ON d.program_id = p.id
      WHERE d.user_id = ${user.id} ORDER BY d.created_at DESC
    `;
    return NextResponse.json({ deployments: rows });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

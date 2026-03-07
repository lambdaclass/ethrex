import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";

export async function GET(req: NextRequest) {
  try {
    await ensureSchema();
    const { searchParams } = req.nextUrl;
    const search = searchParams.get("search");
    const limit = parseInt(searchParams.get("limit") || "50");
    const offset = parseInt(searchParams.get("offset") || "0");

    let rows;
    if (search) {
      const pattern = `%${search}%`;
      ({ rows } = await sql`
        SELECT d.id, d.name, d.chain_id, d.rpc_url, d.status, d.phase,
               d.bridge_address, d.proposer_address, d.created_at,
               p.name as program_name, p.program_id as program_slug, p.category,
               u.name as owner_name
        FROM deployments d
        JOIN programs p ON d.program_id = p.id
        JOIN users u ON d.user_id = u.id
        WHERE d.status = 'active' AND (d.name ILIKE ${pattern} OR p.name ILIKE ${pattern})
        ORDER BY d.created_at DESC LIMIT ${limit} OFFSET ${offset}
      `);
    } else {
      ({ rows } = await sql`
        SELECT d.id, d.name, d.chain_id, d.rpc_url, d.status, d.phase,
               d.bridge_address, d.proposer_address, d.created_at,
               p.name as program_name, p.program_id as program_slug, p.category,
               u.name as owner_name
        FROM deployments d
        JOIN programs p ON d.program_id = p.id
        JOIN users u ON d.user_id = u.id
        WHERE d.status = 'active'
        ORDER BY d.created_at DESC LIMIT ${limit} OFFSET ${offset}
      `);
    }

    return NextResponse.json({ appchains: rows });
  } catch (e) {
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

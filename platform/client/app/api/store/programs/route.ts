import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";

export async function GET(req: NextRequest) {
  try {
    await ensureSchema();
    const { searchParams } = req.nextUrl;
    const category = searchParams.get("category");
    const search = searchParams.get("search");
    const limit = Math.max(0, parseInt(searchParams.get("limit") || "50", 10));
    const offset = Math.max(0, parseInt(searchParams.get("offset") || "0", 10));

    let rows;
    if (category && search) {
      const pattern = `%${search}%`;
      ({ rows } = await sql`
        SELECT * FROM programs WHERE status = 'active' AND category = ${category}
        AND (name ILIKE ${pattern} OR description ILIKE ${pattern} OR program_id ILIKE ${pattern})
        ORDER BY use_count DESC, created_at DESC LIMIT ${limit} OFFSET ${offset}
      `);
    } else if (category) {
      ({ rows } = await sql`
        SELECT * FROM programs WHERE status = 'active' AND category = ${category}
        ORDER BY use_count DESC, created_at DESC LIMIT ${limit} OFFSET ${offset}
      `);
    } else if (search) {
      const pattern = `%${search}%`;
      ({ rows } = await sql`
        SELECT * FROM programs WHERE status = 'active'
        AND (name ILIKE ${pattern} OR description ILIKE ${pattern} OR program_id ILIKE ${pattern})
        ORDER BY use_count DESC, created_at DESC LIMIT ${limit} OFFSET ${offset}
      `);
    } else {
      ({ rows } = await sql`
        SELECT * FROM programs WHERE status = 'active'
        ORDER BY use_count DESC, created_at DESC LIMIT ${limit} OFFSET ${offset}
      `);
    }

    return NextResponse.json({ programs: rows });
  } catch (e) {
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

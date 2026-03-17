import { NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";

export async function GET() {
  try {
    await ensureSchema();
    const { rows } = await sql`
      SELECT * FROM programs WHERE status = 'active'
      ORDER BY use_count DESC, created_at DESC LIMIT 6
    `;
    return NextResponse.json({ programs: rows });
  } catch (e) {
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

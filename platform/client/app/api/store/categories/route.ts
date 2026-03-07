import { NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";

export async function GET() {
  try {
    await ensureSchema();
    const { rows } = await sql`
      SELECT DISTINCT category FROM programs WHERE status = 'active' ORDER BY category
    `;
    return NextResponse.json({ categories: rows.map((r) => r.category) });
  } catch (e) {
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

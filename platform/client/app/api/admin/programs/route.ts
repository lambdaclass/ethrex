import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAdmin } from "@/lib/auth";

export async function GET(req: NextRequest) {
  try {
    await requireAdmin(req);
    await ensureSchema();
    const status = req.nextUrl.searchParams.get("status");

    const { rows } = status
      ? await sql`SELECT * FROM programs WHERE status = ${status} ORDER BY created_at DESC`
      : await sql`SELECT * FROM programs ORDER BY created_at DESC`;

    return NextResponse.json({ programs: rows });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

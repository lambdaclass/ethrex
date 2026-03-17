import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAdmin } from "@/lib/auth";

export async function GET(req: NextRequest) {
  try {
    await requireAdmin(req);
    await ensureSchema();

    const { rows: [stats] } = await sql`
      SELECT
        (SELECT COUNT(*) FROM users) as users,
        (SELECT COUNT(*) FROM programs) as programs,
        (SELECT COUNT(*) FROM programs WHERE status = 'active') as active,
        (SELECT COUNT(*) FROM programs WHERE status = 'pending') as pending,
        (SELECT COUNT(*) FROM deployments) as deployments
    `;

    return NextResponse.json(stats);
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

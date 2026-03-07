import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { requireAdmin } from "@/lib/auth";

export async function GET(req: NextRequest) {
  try {
    await requireAdmin(req);
    await ensureSchema();

    const { rows: [{ count: users }] } = await sql`SELECT COUNT(*) as count FROM users`;
    const { rows: [{ count: programs }] } = await sql`SELECT COUNT(*) as count FROM programs`;
    const { rows: [{ count: active }] } = await sql`SELECT COUNT(*) as count FROM programs WHERE status = 'active'`;
    const { rows: [{ count: pending }] } = await sql`SELECT COUNT(*) as count FROM programs WHERE status = 'pending'`;
    const { rows: [{ count: deployments }] } = await sql`SELECT COUNT(*) as count FROM deployments`;

    return NextResponse.json({ users, programs, active, pending, deployments });
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: String(e) }, { status: 500 });
  }
}

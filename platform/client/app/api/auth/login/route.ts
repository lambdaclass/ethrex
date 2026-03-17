import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { createSession } from "@/lib/auth";
import bcrypt from "bcryptjs";

export async function POST(req: NextRequest) {
  try {
    await ensureSchema();
    const { email, password } = await req.json();

    if (!email || !password) {
      return NextResponse.json({ error: "email and password are required" }, { status: 400 });
    }

    const { rows } = await sql`SELECT * FROM users WHERE email = ${email}`;
    const user = rows[0];
    if (!user || !user.password_hash) {
      return NextResponse.json({ error: "Invalid credentials" }, { status: 401 });
    }

    const valid = await bcrypt.compare(password, user.password_hash as string);
    if (!valid) {
      return NextResponse.json({ error: "Invalid credentials" }, { status: 401 });
    }

    if (user.status !== "active") {
      return NextResponse.json({ error: "Account is suspended" }, { status: 403 });
    }

    const token = await createSession(user.id as string);
    return NextResponse.json({
      token,
      user: { id: user.id, email: user.email, name: user.name, role: user.role, picture: user.picture },
    });
  } catch (e: unknown) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}

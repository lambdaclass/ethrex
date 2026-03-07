import { NextRequest, NextResponse } from "next/server";
import { sql, ensureSchema } from "@/lib/db";
import { createSession } from "@/lib/auth";
import { isValidEmail, isValidPassword, isValidName } from "@/lib/validate";
import bcrypt from "bcryptjs";

export async function POST(req: NextRequest) {
  try {
    await ensureSchema();
    const { email, password, name } = await req.json();

    if (!email || !password || !name) {
      return NextResponse.json({ error: "email, password, and name are required" }, { status: 400 });
    }
    if (!isValidEmail(email)) {
      return NextResponse.json({ error: "Invalid email format" }, { status: 400 });
    }
    if (!isValidPassword(password)) {
      return NextResponse.json({ error: "Password must be 8-128 characters" }, { status: 400 });
    }
    if (!isValidName(name)) {
      return NextResponse.json({ error: "Name must be 1-100 characters" }, { status: 400 });
    }

    const { rows: existing } = await sql`SELECT 1 FROM users WHERE email = ${email}`;
    if (existing.length > 0) {
      return NextResponse.json({ error: "Email already registered" }, { status: 409 });
    }

    const passwordHash = await bcrypt.hash(password, 10);
    const id = crypto.randomUUID();
    const now = Date.now();
    await sql`
      INSERT INTO users (id, email, name, password_hash, auth_provider, created_at)
      VALUES (${id}, ${email}, ${name.trim()}, ${passwordHash}, 'email', ${now})
    `;

    const token = await createSession(id);
    return NextResponse.json({
      token,
      user: { id, email, name: name.trim(), role: "user", picture: null },
    }, { status: 201 });
  } catch (e: unknown) {
    return NextResponse.json({ error: e instanceof Error ? e.message : String(e) }, { status: 500 });
  }
}

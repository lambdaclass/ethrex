/**
 * Server-side auth helpers for API routes.
 * Ported from platform/server/middleware/auth.js + db/sessions.js
 */
import { sql, ensureSchema } from "./db";
import { NextRequest } from "next/server";

const SESSION_TTL_MS = 24 * 60 * 60 * 1000; // 24 hours

export interface SessionUser {
  id: string;
  email: string;
  name: string;
  role: string;
  picture: string | null;
  auth_provider: string;
  status: string;
}

export async function createSession(userId: string): Promise<string> {
  await ensureSchema();
  const token = "ps_" + Array.from(crypto.getRandomValues(new Uint8Array(32)))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
  await sql`INSERT INTO sessions (token, user_id, created_at) VALUES (${token}, ${userId}, ${Date.now()})`;
  return token;
}

export async function destroySession(token: string): Promise<void> {
  await sql`DELETE FROM sessions WHERE token = ${token}`;
}

export async function getSessionUser(req: NextRequest): Promise<{ user: SessionUser; token: string } | null> {
  await ensureSchema();
  const bearer = req.headers.get("authorization")?.replace("Bearer ", "") ||
    req.nextUrl.searchParams.get("token");
  if (!bearer) return null;

  const { rows } = await sql`SELECT user_id, created_at FROM sessions WHERE token = ${bearer}`;
  if (rows.length === 0) return null;

  const session = rows[0];
  if (Date.now() - Number(session.created_at) > SESSION_TTL_MS) {
    await sql`DELETE FROM sessions WHERE token = ${bearer}`;
    return null;
  }

  const { rows: userRows } = await sql`SELECT * FROM users WHERE id = ${session.user_id}`;
  if (userRows.length === 0) {
    await sql`DELETE FROM sessions WHERE token = ${bearer}`;
    return null;
  }

  const user = userRows[0] as unknown as SessionUser;
  if (user.status !== "active") return null;

  return { user, token: bearer };
}

/**
 * Require authentication. Returns user or throws a Response.
 */
export async function requireAuth(req: NextRequest): Promise<SessionUser> {
  const result = await getSessionUser(req);
  if (!result) {
    throw new Response(JSON.stringify({ error: "Authentication required" }), {
      status: 401,
      headers: { "Content-Type": "application/json" },
    });
  }
  return result.user;
}

export async function requireAdmin(req: NextRequest): Promise<SessionUser> {
  const user = await requireAuth(req);
  if (user.role !== "admin") {
    throw new Response(JSON.stringify({ error: "Admin access required" }), {
      status: 403,
      headers: { "Content-Type": "application/json" },
    });
  }
  return user;
}

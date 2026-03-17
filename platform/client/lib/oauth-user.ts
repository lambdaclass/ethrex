import { sql, ensureSchema } from "./db";

interface UserRow {
  id: string;
  email: string;
  name: string;
  role: string;
  picture: string | null;
  auth_provider: string;
  status: string;
}

export async function findOrCreateOAuthUser(
  profile: { email: string; name: string; picture: string | null },
  provider: string
): Promise<UserRow> {
  await ensureSchema();
  const { rows } = await sql`SELECT * FROM users WHERE email = ${profile.email}`;
  if (rows.length > 0) {
    if (rows[0].picture !== profile.picture) {
      await sql`UPDATE users SET picture = ${profile.picture} WHERE id = ${rows[0].id}`;
    }
    return rows[0] as unknown as UserRow;
  }

  const id = crypto.randomUUID();
  const now = Date.now();
  await sql`
    INSERT INTO users (id, email, name, auth_provider, picture, created_at)
    VALUES (${id}, ${profile.email}, ${profile.name}, ${provider}, ${profile.picture}, ${now})
  `;
  const { rows: created } = await sql`SELECT * FROM users WHERE id = ${id}`;
  return created[0] as unknown as UserRow;
}

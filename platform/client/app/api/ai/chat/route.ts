import { NextRequest, NextResponse } from "next/server";
import { requireAuth } from "@/lib/auth";
import {
  checkLimit,
  recordUsage,
  LimitExceededError,
} from "@/lib/token-limiter";

const TOKAMAK_AI_URL = process.env.TOKAMAK_AI_URL || "https://api.ai.tokamak.network/v1/chat/completions";

export async function POST(req: NextRequest) {
  // 1. Authenticate user via session token
  let user;
  try {
    user = await requireAuth(req);
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: "auth_error" }, { status: 401 });
  }

  const dailyLimit = user.daily_ai_limit || 50000;

  // 2. Check daily token limit
  try {
    await checkLimit(user.id, dailyLimit);
  } catch (e) {
    if (e instanceof LimitExceededError) {
      return NextResponse.json(
        { error: "daily_limit_exceeded", usage: e.usage },
        { status: 429 }
      );
    }
    throw e;
  }

  // 3. Parse request body
  let body: Record<string, unknown>;
  try {
    body = await req.json();
  } catch {
    return NextResponse.json({ error: "invalid_json" }, { status: 400 });
  }

  // 4. Proxy to Tokamak AI
  const aiResponse = await fetch(TOKAMAK_AI_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

  if (!aiResponse.ok) {
    const errorText = await aiResponse.text();
    return NextResponse.json(
      { error: "tokamak_ai_error", detail: errorText },
      { status: aiResponse.status }
    );
  }

  const result = await aiResponse.json();

  // 5. Record token usage
  const totalTokens =
    (result.usage?.total_tokens as number) ||
    Math.ceil(JSON.stringify(result).length / 4); // fallback estimate

  const usage = await recordUsage(user.id, totalTokens, dailyLimit);

  // 6. Return response with usage info
  return NextResponse.json({
    ...result,
    _tokamak_usage: usage,
  });
}

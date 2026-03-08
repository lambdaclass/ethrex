import { NextRequest, NextResponse } from "next/server";
import { requireAuth } from "@/lib/auth";
import {
  checkLimit,
  recordUsage,
  LimitExceededError,
  getDefaultDailyLimit,
} from "@/lib/token-limiter";
import { chatCompletion, type ChatMessage } from "@/lib/ai-provider";

export async function POST(req: NextRequest) {
  // 1. Authenticate user via session token
  let user;
  try {
    user = await requireAuth(req);
  } catch (e) {
    if (e instanceof Response) return e;
    return NextResponse.json({ error: "auth_error" }, { status: 401 });
  }

  const dailyLimit = user.daily_ai_limit || getDefaultDailyLimit();

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
  let body: { messages?: ChatMessage[] };
  try {
    body = await req.json();
  } catch {
    return NextResponse.json({ error: "invalid_json" }, { status: 400 });
  }

  if (!body.messages || !Array.isArray(body.messages)) {
    return NextResponse.json({ error: "messages_required" }, { status: 400 });
  }

  // 4. Call AI provider (configured via env vars)
  let aiResult;
  try {
    aiResult = await chatCompletion(body.messages);
  } catch (e) {
    return NextResponse.json(
      { error: "ai_provider_error", detail: String(e) },
      { status: 502 }
    );
  }

  // 5. Record token usage
  const usage = await recordUsage(user.id, aiResult.total_tokens, dailyLimit);

  // 6. Return response in OpenAI-compatible format with usage info
  return NextResponse.json({
    choices: [{ message: { role: "assistant", content: aiResult.content } }],
    usage: { total_tokens: aiResult.total_tokens },
    _tokamak_usage: usage,
  });
}

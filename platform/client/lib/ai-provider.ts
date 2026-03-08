/**
 * AI Provider adapter.
 * Routes chat requests to the configured provider.
 *
 * Env:
 *   TOKAMAK_AI_PROVIDER   — "openai" | "anthropic" | "custom" (default: "openai")
 *   TOKAMAK_AI_API_KEY     — API key for the provider (optional for custom)
 *   TOKAMAK_AI_MODEL       — Model name (default: provider-specific)
 *   TOKAMAK_AI_BASE_URL    — Custom OpenAI-compatible endpoint URL
 *                            (e.g. https://api.ai.tokamak.network)
 *
 * When TOKAMAK_AI_BASE_URL is set, it takes priority and uses OpenAI-compatible
 * format against that URL regardless of TOKAMAK_AI_PROVIDER.
 */

export interface ChatMessage {
  role: string;
  content: string;
}

export interface AiResponse {
  content: string;
  total_tokens: number;
}

type Provider = "openai" | "anthropic" | "custom";

function getBaseUrl(): string | null {
  return process.env.TOKAMAK_AI_BASE_URL || null;
}

function getProvider(): Provider {
  if (getBaseUrl()) return "custom";
  const p = process.env.TOKAMAK_AI_PROVIDER?.toLowerCase();
  if (p === "anthropic") return "anthropic";
  return "openai";
}

function getModel(): string {
  if (process.env.TOKAMAK_AI_MODEL) return process.env.TOKAMAK_AI_MODEL;
  const provider = getProvider();
  if (provider === "custom") return "default";
  if (provider === "anthropic") return "claude-sonnet-4-6";
  return "gpt-4o";
}

function getApiKey(): string | null {
  return process.env.TOKAMAK_AI_API_KEY || null;
}

export async function chatCompletion(messages: ChatMessage[]): Promise<AiResponse> {
  const provider = getProvider();
  if (provider === "anthropic") return callAnthropic(messages);
  return callOpenAICompat(messages);
}

async function callOpenAICompat(messages: ChatMessage[]): Promise<AiResponse> {
  const baseUrl = getBaseUrl() || "https://api.openai.com";
  const url = `${baseUrl.replace(/\/$/, "")}/v1/chat/completions`;
  const apiKey = getApiKey();

  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (apiKey) {
    headers["Authorization"] = `Bearer ${apiKey}`;
  }

  const response = await fetch(url, {
    method: "POST",
    headers,
    body: JSON.stringify({
      model: getModel(),
      messages,
      max_tokens: 4096,
    }),
  });

  if (!response.ok) {
    const errorText = await response.text();
    throw new Error(`AI error (${response.status}): ${errorText}`);
  }

  const result = await response.json();
  const content = result.choices?.[0]?.message?.content || "";
  const total_tokens = result.usage?.total_tokens || Math.ceil(content.length / 4);

  return { content, total_tokens };
}

async function callAnthropic(messages: ChatMessage[]): Promise<AiResponse> {
  const apiKey = getApiKey();
  if (!apiKey) throw new Error("TOKAMAK_AI_API_KEY is required for Anthropic");

  const systemMsg = messages.find((m) => m.role === "system");
  const chatMessages = messages.filter((m) => m.role !== "system");

  const body: Record<string, unknown> = {
    model: getModel(),
    max_tokens: 4096,
    messages: chatMessages.map((m) => ({ role: m.role, content: m.content })),
  };
  if (systemMsg) {
    body.system = systemMsg.content;
  }

  const response = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "x-api-key": apiKey,
      "anthropic-version": "2023-06-01",
    },
    body: JSON.stringify(body),
  });

  if (!response.ok) {
    const errorText = await response.text();
    throw new Error(`Anthropic error (${response.status}): ${errorText}`);
  }

  const result = await response.json();
  const content = result.content?.[0]?.text || "";
  const total_tokens =
    (result.usage?.input_tokens || 0) + (result.usage?.output_tokens || 0) ||
    Math.ceil(content.length / 4);

  return { content, total_tokens };
}

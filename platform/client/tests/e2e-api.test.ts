/**
 * E2E API tests — runs against a live Next.js dev server.
 *
 * Prerequisites:
 *   1. DATABASE_URL must be set (Neon Postgres or local Postgres)
 *   2. Dev server running (npm run dev)
 *
 * Run: TEST_BASE_URL=http://localhost:3099 npx tsx tests/e2e-api.test.ts
 */

(async () => {
  const BASE = process.env.TEST_BASE_URL || "http://localhost:3000";

  let passed = 0;
  let failed = 0;
  let sessionToken = "";
  const testEmail = `e2e-${Date.now()}@test.local`;
  const testPassword = "testpassword123";
  const testName = "E2E Tester";

  async function test(name: string, fn: () => Promise<void>) {
    try {
      await fn();
      passed++;
      console.log(`  PASS: ${name}`);
    } catch (e) {
      failed++;
      console.error(`  FAIL: ${name} — ${e}`);
    }
  }

  function assert(condition: boolean, msg: string) {
    if (!condition) throw new Error(msg);
  }

  async function api(path: string, opts?: RequestInit) {
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      ...(opts?.headers as Record<string, string> || {}),
    };
    if (sessionToken) {
      headers["Authorization"] = `Bearer ${sessionToken}`;
    }
    const res = await fetch(`${BASE}${path}`, { ...opts, headers });
    const data = await res.json();
    return { status: res.status, data };
  }

  // ---- Health ----
  console.log("\n=== Health ===");

  await test("GET /api/health", async () => {
    const { status, data } = await api("/api/health");
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.status === "ok", `expected ok, got ${data.status}`);
  });

  // ---- Auth ----
  console.log("\n=== Auth ===");

  await test("POST /api/auth/signup — creates user", async () => {
    const { status, data } = await api("/api/auth/signup", {
      method: "POST",
      body: JSON.stringify({ email: testEmail, password: testPassword, name: testName }),
    });
    assert(status === 201, `expected 201, got ${status}: ${JSON.stringify(data)}`);
    assert(data.token.startsWith("ps_"), "token should start with ps_");
    assert(data.user.email === testEmail, "email mismatch");
    sessionToken = data.token;
  });

  await test("POST /api/auth/signup — duplicate email rejected", async () => {
    const { status } = await api("/api/auth/signup", {
      method: "POST",
      body: JSON.stringify({ email: testEmail, password: testPassword, name: testName }),
    });
    assert(status === 409, `expected 409, got ${status}`);
  });

  await test("POST /api/auth/login — correct password", async () => {
    const { status, data } = await api("/api/auth/login", {
      method: "POST",
      body: JSON.stringify({ email: testEmail, password: testPassword }),
    });
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.token.startsWith("ps_"), "token should start with ps_");
    sessionToken = data.token;
  });

  await test("POST /api/auth/login — wrong password", async () => {
    const { status } = await api("/api/auth/login", {
      method: "POST",
      body: JSON.stringify({ email: testEmail, password: "wrongpassword" }),
    });
    assert(status === 401, `expected 401, got ${status}`);
  });

  await test("GET /api/auth/me — returns current user", async () => {
    const { status, data } = await api("/api/auth/me");
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.email === testEmail, "email mismatch");
    assert(data.name === testName, "name mismatch");
  });

  await test("GET /api/auth/me — 401 without token", async () => {
    const saved = sessionToken;
    sessionToken = "";
    const { status } = await api("/api/auth/me");
    sessionToken = saved;
    assert(status === 401, `expected 401, got ${status}`);
  });

  await test("GET /api/auth/providers — returns providers", async () => {
    const { status, data } = await api("/api/auth/providers");
    assert(status === 200, `expected 200, got ${status}`);
    assert(typeof data.google === "boolean", "google should be boolean");
    assert(typeof data.naver === "boolean", "naver should be boolean");
    assert(typeof data.kakao === "boolean", "kakao should be boolean");
  });

  await test("PUT /api/auth/profile — updates name", async () => {
    const { status, data } = await api("/api/auth/profile", {
      method: "PUT",
      body: JSON.stringify({ name: "Updated Name" }),
    });
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.user.name === "Updated Name", "name not updated");
  });

  // ---- Store (public) ----
  console.log("\n=== Store ===");

  await test("GET /api/store/programs — lists programs", async () => {
    const { status, data } = await api("/api/store/programs");
    assert(status === 200, `expected 200, got ${status}`);
    assert(Array.isArray(data.programs), "programs should be array");
  });

  await test("GET /api/store/categories — lists categories", async () => {
    const { status, data } = await api("/api/store/categories");
    assert(status === 200, `expected 200, got ${status}`);
    assert(Array.isArray(data.categories), "categories should be array");
  });

  await test("GET /api/store/featured — lists featured", async () => {
    const { status, data } = await api("/api/store/featured");
    assert(status === 200, `expected 200, got ${status}`);
    assert(Array.isArray(data.programs), "programs should be array");
  });

  await test("GET /api/store/appchains — lists appchains", async () => {
    const { status, data } = await api("/api/store/appchains");
    assert(status === 200, `expected 200, got ${status}`);
    assert(Array.isArray(data.appchains), "appchains should be array");
  });

  // ---- Programs (authenticated) ----
  console.log("\n=== Programs ===");

  let programDbId = "";
  const testProgramId = `e2e-prog-${Date.now()}`;

  await test("POST /api/programs — creates program", async () => {
    const { status, data } = await api("/api/programs", {
      method: "POST",
      body: JSON.stringify({
        programId: testProgramId,
        name: "E2E Test Program",
        description: "Test program for e2e",
        category: "general",
      }),
    });
    assert(status === 201, `expected 201, got ${status}: ${JSON.stringify(data)}`);
    assert(data.program.program_id === testProgramId, "programId mismatch");
    programDbId = data.program.id;
  });

  await test("GET /api/programs — lists my programs", async () => {
    const { status, data } = await api("/api/programs");
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.programs.some((p: { id: string }) => p.id === programDbId), "should contain created program");
  });

  await test("GET /api/programs/[id] — gets program", async () => {
    const { status, data } = await api(`/api/programs/${programDbId}`);
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.program.id === programDbId, "id mismatch");
  });

  await test("PUT /api/programs/[id] — updates program", async () => {
    const { status, data } = await api(`/api/programs/${programDbId}`, {
      method: "PUT",
      body: JSON.stringify({ name: "Updated Program" }),
    });
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.program.name === "Updated Program", "name not updated");
  });

  // ---- Deployments (authenticated) ----
  console.log("\n=== Deployments ===");

  let deploymentId = "";

  await test("Find official program for deployment", async () => {
    const { status, data } = await api("/api/store/programs?search=evm-l2");
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.programs.length > 0, "should find evm-l2");
  });

  await test("POST /api/deployments — creates deployment", async () => {
    const { data: storeData } = await api("/api/store/programs?search=evm-l2");
    const evmL2 = storeData.programs[0];

    const { status, data } = await api("/api/deployments", {
      method: "POST",
      body: JSON.stringify({
        programId: evmL2.id,
        name: "E2E Test Deployment",
        chainId: 12345,
      }),
    });
    assert(status === 201, `expected 201, got ${status}: ${JSON.stringify(data)}`);
    assert(data.deployment.name === "E2E Test Deployment", "name mismatch");
    deploymentId = data.deployment.id;
  });

  await test("GET /api/deployments — lists my deployments", async () => {
    const { status, data } = await api("/api/deployments");
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.deployments.some((d: { id: string }) => d.id === deploymentId), "should contain created deployment");
  });

  await test("GET /api/deployments/[id] — gets deployment", async () => {
    const { status, data } = await api(`/api/deployments/${deploymentId}`);
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.deployment.id === deploymentId, "id mismatch");
  });

  await test("PUT /api/deployments/[id] — updates deployment", async () => {
    const { status, data } = await api(`/api/deployments/${deploymentId}`, {
      method: "PUT",
      body: JSON.stringify({ name: "Updated Deployment", chain_id: 99999 }),
    });
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.deployment.name === "Updated Deployment", "name not updated");
  });

  await test("POST /api/deployments/[id]/activate — activates", async () => {
    const { status, data } = await api(`/api/deployments/${deploymentId}/activate`, { method: "POST" });
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.deployment.status === "active", "should be active");
  });

  // ---- AI Proxy ----
  console.log("\n=== AI Proxy ===");

  await test("GET /api/ai/usage — returns usage for logged-in user", async () => {
    const { status, data } = await api("/api/ai/usage");
    assert(status === 200, `expected 200, got ${status}: ${JSON.stringify(data)}`);
    assert(typeof data.used === "number", "used should be number");
    assert(typeof data.limit === "number", "limit should be number");
  });

  await test("GET /api/ai/usage — 401 without auth", async () => {
    const res = await fetch(`${BASE}/api/ai/usage`);
    assert(res.status === 401, `expected 401, got ${res.status}`);
  });

  await test("POST /api/ai/chat — 401 without auth", async () => {
    const res = await fetch(`${BASE}/api/ai/chat`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ messages: [{ role: "user", content: "hi" }] }),
    });
    assert(res.status === 401, `expected 401, got ${res.status}`);
  });

  // ---- Desktop Auth Flow (PKCE) ----
  console.log("\n=== Desktop Auth (PKCE) ===");

  let desktopCode = "";
  const codeVerifier = "test_verifier_" + Date.now();

  // Compute SHA-256 hex of verifier for code_challenge
  async function sha256hex(input: string): Promise<string> {
    const encoder = new TextEncoder();
    const data = encoder.encode(input);
    const hashBuffer = await crypto.subtle.digest("SHA-256", data);
    return Array.from(new Uint8Array(hashBuffer))
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
  }

  const codeChallenge = await sha256hex(codeVerifier);

  await test("POST /api/auth/desktop-code — generates code with PKCE", async () => {
    const { status, data } = await api("/api/auth/desktop-code", {
      method: "POST",
      body: JSON.stringify({ code_challenge: codeChallenge }),
    });
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.code.startsWith("dc_"), `code should start with dc_, got ${data.code}`);
    assert(data.expires_in === 300, `expires_in should be 300, got ${data.expires_in}`);
    desktopCode = data.code;
  });

  await test("POST /api/auth/desktop-code — rejects without code_challenge", async () => {
    const { status, data } = await api("/api/auth/desktop-code", { method: "POST", body: "{}" });
    assert(status === 400, `expected 400, got ${status}`);
    assert(data.error === "code_challenge_required", `should be code_challenge_required, got ${data.error}`);
  });

  await test("GET /api/auth/desktop-token — pending before login", async () => {
    const { status, data } = await api(`/api/auth/desktop-token?code=${desktopCode}&code_verifier=${codeVerifier}`);
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.status === "pending", `should be pending, got ${data.status}`);
  });

  await test("GET /api/auth/desktop-token — rejects wrong verifier", async () => {
    const { status, data } = await api(`/api/auth/desktop-token?code=${desktopCode}&code_verifier=wrong_verifier`);
    assert(status === 403, `expected 403, got ${status}`);
    assert(data.error === "invalid_verifier", `should be invalid_verifier, got ${data.error}`);
  });

  await test("PUT /api/auth/desktop-code — links token (authenticated)", async () => {
    // Login to get a fresh token
    const loginRes = await api("/api/auth/login", {
      method: "POST",
      body: JSON.stringify({ email: testEmail, password: testPassword }),
    });
    const freshToken = loginRes.data.token;
    sessionToken = freshToken;

    const { status, data } = await api("/api/auth/desktop-code", {
      method: "PUT",
      body: JSON.stringify({ code: desktopCode }),
    });
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.ok === true, "should return ok");
  });

  await test("PUT /api/auth/desktop-code — rejects without auth", async () => {
    const saved = sessionToken;
    sessionToken = "";
    const { status } = await api("/api/auth/desktop-code", {
      method: "PUT",
      body: JSON.stringify({ code: desktopCode }),
    });
    sessionToken = saved;
    assert(status === 401, `expected 401, got ${status}`);
  });

  await test("GET /api/auth/desktop-token — ready with correct verifier", async () => {
    const { status, data } = await api(`/api/auth/desktop-token?code=${desktopCode}&code_verifier=${codeVerifier}`);
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.status === "ready", `should be ready, got ${data.status}`);
    assert(data.token.startsWith("ps_"), `token should start with ps_, got ${data.token}`);
  });

  await test("GET /api/auth/desktop-token — code consumed after retrieval", async () => {
    const { status, data } = await api(`/api/auth/desktop-token?code=${desktopCode}&code_verifier=${codeVerifier}`);
    assert(status === 404, `expected 404 after consumption, got ${status}`);
    assert(data.error === "invalid_code", `should be invalid_code, got ${data.error}`);
  });

  await test("GET /api/auth/desktop-token — missing verifier returns 400", async () => {
    const { status, data } = await api("/api/auth/desktop-token?code=dc_test");
    assert(status === 400, `expected 400, got ${status}`);
    assert(data.error === "code_and_verifier_required", `should be code_and_verifier_required, got ${data.error}`);
  });

  // ---- Cleanup ----
  console.log("\n=== Cleanup ===");

  await test("DELETE /api/deployments/[id] — deletes deployment", async () => {
    const { status, data } = await api(`/api/deployments/${deploymentId}`, { method: "DELETE" });
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.ok === true, "should return ok");
  });

  await test("DELETE /api/programs/[id] — soft deletes program", async () => {
    const { status, data } = await api(`/api/programs/${programDbId}`, { method: "DELETE" });
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.program.status === "disabled", "should be disabled");
  });

  await test("POST /api/auth/logout — destroys session", async () => {
    const { status, data } = await api("/api/auth/logout", { method: "POST" });
    assert(status === 200, `expected 200, got ${status}`);
    assert(data.ok === true, "should return ok");
  });

  await test("GET /api/auth/me — 401 after logout", async () => {
    const { status } = await api("/api/auth/me");
    assert(status === 401, `expected 401, got ${status}`);
  });

  // ---- Summary ----
  console.log(`\n=== E2E Results: ${passed} passed, ${failed} failed ===\n`);
  process.exit(failed > 0 ? 1 : 0);
})();

const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://localhost:5001";

async function apiFetch(path: string, options: RequestInit = {}) {
  const token = typeof window !== "undefined" ? localStorage.getItem("session_token") : null;
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...(options.headers as Record<string, string>),
  };
  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }

  const res = await fetch(`${API_URL}${path}`, { ...options, headers });
  const data = await res.json();
  if (!res.ok) throw new Error(data.error || "Request failed");
  return data;
}

// Auth
export const authApi = {
  signup: (email: string, password: string, name: string) =>
    apiFetch("/api/auth/signup", { method: "POST", body: JSON.stringify({ email, password, name }) }),
  login: (email: string, password: string) =>
    apiFetch("/api/auth/login", { method: "POST", body: JSON.stringify({ email, password }) }),
  google: (idToken: string) =>
    apiFetch("/api/auth/google", { method: "POST", body: JSON.stringify({ idToken }) }),
  naver: (code: string, state: string) =>
    apiFetch("/api/auth/naver", { method: "POST", body: JSON.stringify({ code, state }) }),
  kakao: (code: string, redirectUri: string) =>
    apiFetch("/api/auth/kakao", { method: "POST", body: JSON.stringify({ code, redirectUri }) }),
  me: () => apiFetch("/api/auth/me"),
  updateProfile: (body: { name: string }) =>
    apiFetch("/api/auth/profile", { method: "PUT", body: JSON.stringify(body) }),
  logout: () => apiFetch("/api/auth/logout", { method: "POST" }),
  providers: () => apiFetch("/api/auth/providers"),
  googleClientId: () => apiFetch("/api/auth/google-client-id"),
  naverClientId: () => apiFetch("/api/auth/naver-client-id"),
  kakaoClientId: () => apiFetch("/api/auth/kakao-client-id"),
};

// Store (public)
export const storeApi = {
  programs: async (params?: { category?: string; search?: string }) => {
    const qs = new URLSearchParams(params as Record<string, string>).toString();
    const data = await apiFetch(`/api/store/programs${qs ? `?${qs}` : ""}`);
    return data.programs;
  },
  program: async (id: string) => {
    const data = await apiFetch(`/api/store/programs/${id}`);
    return data.program;
  },
  categories: async () => {
    const data = await apiFetch("/api/store/categories");
    return data.categories;
  },
  featured: async () => {
    const data = await apiFetch("/api/store/featured");
    return data.programs;
  },
};

// File upload helper (multipart/form-data)
async function apiUpload(path: string, formData: FormData) {
  const token = typeof window !== "undefined" ? localStorage.getItem("session_token") : null;
  const headers: Record<string, string> = {};
  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }
  // Do NOT set Content-Type — browser sets it with boundary automatically
  const res = await fetch(`${API_URL}${path}`, { method: "POST", headers, body: formData });
  const data = await res.json();
  if (!res.ok) throw new Error(data.error || "Upload failed");
  return data;
}

// Programs (creator, auth required)
export const programsApi = {
  list: async () => {
    const data = await apiFetch("/api/programs");
    return data.programs;
  },
  get: async (id: string) => {
    const data = await apiFetch(`/api/programs/${id}`);
    return data.program;
  },
  create: async (body: { programId: string; name: string; description?: string; category?: string }) => {
    const data = await apiFetch("/api/programs", { method: "POST", body: JSON.stringify(body) });
    return data.program;
  },
  update: async (id: string, body: Record<string, unknown>) => {
    const data = await apiFetch(`/api/programs/${id}`, { method: "PUT", body: JSON.stringify(body) });
    return data.program;
  },
  remove: async (id: string) => {
    const data = await apiFetch(`/api/programs/${id}`, { method: "DELETE" });
    return data.program;
  },
  uploadElf: async (id: string, file: File) => {
    const formData = new FormData();
    formData.append("elf", file);
    return apiUpload(`/api/programs/${id}/upload/elf`, formData);
  },
  uploadVk: async (id: string, files: { sp1?: File; risc0?: File }) => {
    const formData = new FormData();
    if (files.sp1) formData.append("vk_sp1", files.sp1);
    if (files.risc0) formData.append("vk_risc0", files.risc0);
    return apiUpload(`/api/programs/${id}/upload/vk`, formData);
  },
  versions: async (id: string) => {
    const data = await apiFetch(`/api/programs/${id}/versions`);
    return data.versions;
  },
};

// Deployments (auth required)
export const deploymentsApi = {
  list: async () => {
    const data = await apiFetch("/api/deployments");
    return data.deployments;
  },
  get: async (id: string) => {
    const data = await apiFetch(`/api/deployments/${id}`);
    return data.deployment;
  },
  create: async (body: { programId: string; name: string; chainId?: number; rpcUrl?: string }) => {
    const data = await apiFetch("/api/deployments", { method: "POST", body: JSON.stringify(body) });
    return data.deployment;
  },
  update: async (id: string, body: Record<string, unknown>) => {
    const data = await apiFetch(`/api/deployments/${id}`, { method: "PUT", body: JSON.stringify(body) });
    return data.deployment;
  },
  remove: async (id: string) => {
    await apiFetch(`/api/deployments/${id}`, { method: "DELETE" });
  },
  activate: async (id: string) => {
    const data = await apiFetch(`/api/deployments/${id}/activate`, { method: "POST" });
    return data.deployment;
  },
};

// Admin
export const adminApi = {
  programs: async (status?: string) => {
    const data = await apiFetch(`/api/admin/programs${status ? `?status=${status}` : ""}`);
    return data.programs;
  },
  program: async (id: string) => {
    return apiFetch(`/api/admin/programs/${id}`);
  },
  approve: async (id: string) => {
    const data = await apiFetch(`/api/admin/programs/${id}/approve`, { method: "PUT" });
    return data.program;
  },
  reject: async (id: string) => {
    const data = await apiFetch(`/api/admin/programs/${id}/reject`, { method: "PUT" });
    return data.program;
  },
  stats: () => apiFetch("/api/admin/stats"),
  users: async () => {
    const data = await apiFetch("/api/admin/users");
    return data.users;
  },
  changeRole: async (id: string, role: string) => {
    const data = await apiFetch(`/api/admin/users/${id}/role`, { method: "PUT", body: JSON.stringify({ role }) });
    return data.user;
  },
  suspendUser: async (id: string) => {
    const data = await apiFetch(`/api/admin/users/${id}/suspend`, { method: "PUT" });
    return data.user;
  },
  activateUser: async (id: string) => {
    const data = await apiFetch(`/api/admin/users/${id}/activate`, { method: "PUT" });
    return data.user;
  },
  deployments: async () => {
    const data = await apiFetch("/api/admin/deployments");
    return data.deployments;
  },
};

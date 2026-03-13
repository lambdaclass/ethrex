const API_URL = process.env.NEXT_PUBLIC_API_URL || "";

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
  appchains: async (params?: { search?: string }) => {
    const qs = params?.search ? `?search=${encodeURIComponent(params.search)}` : "";
    const data = await apiFetch(`/api/store/appchains${qs}`);
    return data.appchains;
  },
  appchain: async (id: string) => {
    const data = await apiFetch(`/api/store/appchains/${id}`);
    return data.appchain;
  },
  appchainRpc: async (id: string, method: string, params: unknown[] = []) => {
    try {
      const res = await fetch(`${API_URL}/api/store/appchains/${id}/rpc-proxy`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ method, params }),
      });
      if (!res.ok) return null;
      const data = await res.json();
      return data.result ?? null;
    } catch {
      return null;
    }
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
  create: async (body: { programId: string; name: string; chainId?: number; rpcUrl?: string; config?: Record<string, unknown> }) => {
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

// Social (public + wallet auth)
type Wallet = { address: string; signature: string };

function walletHeaders(wallet: Wallet): Record<string, string> {
  return { "x-wallet-address": wallet.address, "x-wallet-signature": wallet.signature };
}

export const socialApi = {
  getReviews: async (id: string, walletAddress?: string) => {
    const headers: Record<string, string> = {};
    if (walletAddress) headers["x-wallet-address"] = walletAddress;
    return apiFetch(`/api/store/appchains/${id}/reviews`, { headers });
  },
  createReview: async (id: string, body: { rating: number; content: string }, wallet: Wallet) => {
    const data = await apiFetch(`/api/store/appchains/${id}/reviews`, {
      method: "POST", headers: walletHeaders(wallet), body: JSON.stringify(body),
    });
    return data.review;
  },
  deleteReview: async (deploymentId: string, reviewId: string, wallet: Wallet) => {
    return apiFetch(`/api/store/appchains/${deploymentId}/reviews/${reviewId}`, {
      method: "DELETE", headers: walletHeaders(wallet),
    });
  },
  getComments: async (id: string, walletAddress?: string) => {
    const headers: Record<string, string> = {};
    if (walletAddress) headers["x-wallet-address"] = walletAddress;
    return apiFetch(`/api/store/appchains/${id}/comments`, { headers });
  },
  createComment: async (id: string, body: { content: string; parentId?: string }, wallet: Wallet) => {
    const data = await apiFetch(`/api/store/appchains/${id}/comments`, {
      method: "POST", headers: walletHeaders(wallet), body: JSON.stringify(body),
    });
    return data.comment;
  },
  deleteComment: async (deploymentId: string, commentId: string, wallet: Wallet) => {
    return apiFetch(`/api/store/appchains/${deploymentId}/comments/${commentId}`, {
      method: "DELETE", headers: walletHeaders(wallet),
    });
  },
  toggleReaction: async (id: string, body: { targetType: string; targetId: string }, wallet: Wallet) => {
    return apiFetch(`/api/store/appchains/${id}/reactions`, {
      method: "POST", headers: walletHeaders(wallet), body: JSON.stringify(body),
    });
  },
};

// Bookmarks (account-based auth)
export const bookmarkApi = {
  toggle: (id: string) =>
    apiFetch(`/api/store/appchains/${id}/bookmark`, { method: "POST" }),
  list: async (): Promise<string[]> => {
    const data = await apiFetch("/api/store/bookmarks");
    return data.bookmarks;
  },
};

// Announcements (owner wallet create/delete, public read)
export const announcementApi = {
  list: (id: string) => apiFetch(`/api/store/appchains/${id}/announcements`),
  create: (id: string, body: { title: string; content: string; pinned?: boolean }, wallet: Wallet) =>
    apiFetch(`/api/store/appchains/${id}/announcements`, {
      method: "POST",
      headers: walletHeaders(wallet),
      body: JSON.stringify(body),
    }),
  update: (deploymentId: string, announcementId: string, body: { title: string; content: string; pinned?: boolean }, wallet: Wallet) =>
    apiFetch(`/api/store/appchains/${deploymentId}/announcements/${announcementId}`, {
      method: "PUT",
      headers: walletHeaders(wallet),
      body: JSON.stringify(body),
    }),
  delete: (deploymentId: string, announcementId: string, wallet: Wallet) =>
    apiFetch(`/api/store/appchains/${deploymentId}/announcements/${announcementId}`, {
      method: "DELETE",
      headers: walletHeaders(wallet),
    }),
};

// Hosts and filesystem APIs moved to Desktop local-server

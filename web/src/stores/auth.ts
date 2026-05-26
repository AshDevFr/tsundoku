import { create } from "zustand";
import { persist } from "zustand/middleware";

// Single-user admin token. Persisted to localStorage so the operator only
// enters it once per browser. Read endpoints don't need it in the default
// config; write endpoints (link / reject / retry / refresh) do.
interface AdminAuthState {
  token: string | null;
  setToken: (token: string | null) => void;
  clear: () => void;
}

export const useAdminAuth = create<AdminAuthState>()(
  persist(
    (set) => ({
      token: null,
      setToken: (token) => set({ token: token?.trim() ? token : null }),
      clear: () => set({ token: null }),
    }),
    {
      name: "tsundoku.admin-token.v1",
    },
  ),
);

export function currentAdminToken(): string | null {
  return useAdminAuth.getState().token;
}

import { create } from "zustand";

// Example Zustand store. Replace with real app state; add persist middleware
// (zustand/middleware) when you need localStorage-backed slices.
interface UiState {
  count: number;
  increment: () => void;
}

export const useUiStore = create<UiState>((set) => ({
  count: 0,
  increment: () => set((s) => ({ count: s.count + 1 })),
}));

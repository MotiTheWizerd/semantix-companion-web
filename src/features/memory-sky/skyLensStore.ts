import { create } from "zustand";

import type { SkyStats } from "./engine/MemorySky";

// The lens the sky is looked through, and what the sky sees through it.
//
// Two siblings share it: the sky view applies the filter and reports what is
// on screen; the sidebar's legend shows that and edits the filter. They are
// siblings under the shell, not parent and child, so props cannot carry it —
// and the sidebar is on the chat's bundle, so nothing here may cost three.js
// (the engine import above is type-only and erased).
//
// The lens is a way of LOOKING, not a property of one mind: it survives a
// companion switch, and it survives leaving the view and coming back.

/** The loaded mind's own numbers — the graph door's `stats`. */
export interface SkyMindStats {
  nodes: number;
  link_edges: number;
  semantic_edges: number;
  dangling_links: number;
}

export type SkyTypeCount = readonly [type: string, count: number];

const NO_TYPES: ReadonlySet<string> = new Set();

interface SkyLensState {
  // ── the lens ──
  /** Types put out. Their orbs and bolts sink to ghost in the engine. */
  hiddenTypes: ReadonlySet<string>;
  /** Memories below this importance sink. */
  minImportance: number;
  /** Archived memories drawn as ghosts — a reload, not a lighting pass. */
  showArchived: boolean;

  // ── what the sky reports ──
  /** Types in the loaded mind with their counts, in legend order. Empty
   *  while no mind is loaded. */
  typeCounts: readonly SkyTypeCount[];
  mind: SkyMindStats | null;
  /** The engine's running numbers — fps, how many orbs are lit, settled. */
  sky: SkyStats | null;

  toggleType: (type: string) => void;
  /** Only this type — or, if it already stands alone, everything back. */
  soloType: (type: string) => void;
  showAllTypes: () => void;
  setMinImportance: (value: number) => void;
  setShowArchived: (value: boolean) => void;

  reportMind: (typeCounts: readonly SkyTypeCount[], mind: SkyMindStats | null) => void;
  reportSky: (sky: SkyStats | null) => void;
}

export const useSkyLensStore = create<SkyLensState>((set, get) => ({
  hiddenTypes: NO_TYPES,
  minImportance: 0,
  showArchived: false,

  typeCounts: [],
  mind: null,
  sky: null,

  toggleType: (type) =>
    set((state) => {
      const next = new Set(state.hiddenTypes);
      if (next.has(type)) next.delete(type);
      else next.add(type);
      return { hiddenTypes: next };
    }),

  soloType: (type) => {
    const { hiddenTypes, typeCounts } = get();
    const allTypes = typeCounts.map(([t]) => t);
    const alone = !hiddenTypes.has(type) && allTypes.every((t) => t === type || hiddenTypes.has(t));
    set({ hiddenTypes: alone ? NO_TYPES : new Set(allTypes.filter((t) => t !== type)) });
  },

  showAllTypes: () => set({ hiddenTypes: NO_TYPES }),
  setMinImportance: (minImportance) => set({ minImportance }),
  setShowArchived: (showArchived) => set({ showArchived }),

  reportMind: (typeCounts, mind) => set({ typeCounts, mind }),
  reportSky: (sky) => set({ sky }),
}));

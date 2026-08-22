// The reflex contract — ported from the studio module (s481). A reflex is one
// independent memory sense that fires on the pre-send seam: it reads the
// context, optionally asks the organ, and returns a contribution — or null
// when it has nothing to say. Reflexes never see each other; cross-reflex
// concerns (dedupe, ordering, the wrapper block) live in the composer.

import type { MemoryHit } from "../organService";

export interface ReflexCtx {
  conversationId: string | null;
  /** Companion's memory agent on the organ. */
  agentId: string;
  /** The user's outgoing message. */
  text: string;
  /** The assistant's last reply in this conversation — the mirror leg's cue. */
  lastAssistantText: string | null;
  nowMs: number;
  /** Stamp of the user's PREVIOUS message, null on the conversation's first. */
  lastUserAtMs: number | null;
}

export type ReflexContribution =
  | {
      kind: "hits";
      hits: MemoryHit[];
      /** Mirror hits were cued by the agent's own words, not the user's. */
      mirror?: boolean;
      /** Max hits this contribution may land AFTER dedupe. */
      cap?: number;
      /** false = the organ answered on the keyword leg alone. */
      vectorLeg?: boolean;
    }
  | {
      /** A ready block that rides ahead of the memory block (e.g. time). */
      kind: "block";
      text: string;
    };

export interface MemoryReflex {
  /** Stable id — also the pref-key suffix (memory.reflex.<id>). */
  id: string;
  /** Human name for the settings UI. */
  label: string;
  /** One line for the settings UI — what sense this reflex adds. */
  description: string;
  defaultOn: boolean;
  run(ctx: ReflexCtx): Promise<ReflexContribution | null>;
}

/** What one pre-send pass produced — the injection plus instrument data. */
export interface ReflexRunReport {
  /** Everything that rides ahead of the user's message; '' = nothing. */
  injection: string;
  hits: MemoryHit[];
  mirrorHits: MemoryHit[];
  /** null = no recall ran this turn. */
  vectorLeg: boolean | null;
  /** Reflexes that ran (enabled) this pass. */
  ran: string[];
  /** Per-reflex failures — reflexes fail open, the send never blocks. */
  errors: { reflexId: string; message: string }[];
}

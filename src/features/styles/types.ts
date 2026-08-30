// The style contract, spoken by both halves of the app.
//
// A style is a reusable VOICE — a name, an optional trait sheet (the "style
// card"), and real example exchanges that teach the voice by demonstration.
// It lives in a library of its own; companions hold a reference to one, so a
// single style can dress many companions and editing it re-dresses them all.
//
// The line the whole feature holds: a style transfers a way of speaking,
// never an identity. The exchanges say "user" and "companion" — never
// "assistant" — and the model wearing one stays honest about what it is.

export interface Style {
  id: string;
  name: string;
  description: string | null;
  /** The distilled trait sheet. Optional — exemplars alone still work. */
  styleCard: string | null;
  createdAt: number;
  updatedAt: number;
  /** How many exchanges the style holds, without loading them. */
  exemplarCount: number;
}

export interface StyleExemplar {
  id: string;
  position: number;
  userText: string;
  companionText: string;
  /** YYYY-MM of the source exchange, when the export dated it. */
  era: string | null;
}

export interface StyleExemplarInput {
  userText: string;
  companionText: string;
  era?: string | null;
}

export interface CreateStyleInput {
  name: string;
  description?: string | null;
  styleCard?: string | null;
  exemplars: StyleExemplarInput[];
}

export interface UpdateStyleInput {
  styleId: string;
  name: string;
  description?: string | null;
  styleCard?: string | null;
  /** Omit to keep the stored exchanges untouched; send to replace them. */
  exemplars?: StyleExemplarInput[];
}

export type StyleChangedEvent =
  | { kind: "created"; style: Style }
  | { kind: "updated"; style: Style }
  | { kind: "deleted"; styleId: string };

// --- The harvest: mining an export for the voice the user misses ---

export interface ModelCount {
  slug: string;
  /** Chats where this model spoke at least once. */
  chatCount: number;
}

export interface StyleSourceInspection {
  source: "claude" | "chatgpt";
  conversationCount: number;
  earliestMs: number;
  latestMs: number;
  /** Empty for Claude exports — that format never says which model spoke. */
  models: ModelCount[];
}

export interface HarvestStyleInput {
  path: string;
  modelSlug?: string | null;
  fromMs?: number | null;
  toMs?: number | null;
  target?: number;
}

export interface HarvestedPair {
  userText: string;
  companionText: string;
  era: string | null;
  chatTitle: string;
}

export interface HarvestResult {
  /** Every exchange that matched, before the diverse selection. */
  matchedPairs: number;
  pairs: HarvestedPair[];
}

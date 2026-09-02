// The memory organ client — Companion's door to /api/v1/memory on :8002.
// Every organ call goes THROUGH RUST (invoke): the organ's CORS allowlist
// knows the studio's origin, not Companion's, so a webview fetch dies with
// WebKit's opaque "Load failed" (proven s484). Rust attaches the bearer from
// the vault; the token never rides through this module except when saved.
// The TS types below remain the contract both products speak.

import { invoke } from "@tauri-apps/api/core";

export interface MemoryAgent {
  agent_id: string;
  name: string;
  description: string;
  memory_count: number;
  created_at: string;
  updated_at: string;
}

export interface MemoryRecord {
  id: string;
  agent_id: string;
  name: string;
  description: string;
  body: string;
  mem_type: string;
  importance: number;
  project_tag: string | null;
  links: string[];
  access_count: number;
  archived_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface MemoryHit {
  memory: MemoryRecord;
  score: number;
}

export interface RecallResult {
  hits: MemoryHit[];
  /** false = the embedder failed and recall ran on the keyword leg alone. */
  vector_leg: boolean;
}

let cachedToken: string | null | undefined;

export async function loadAccountToken(): Promise<string | null> {
  if (cachedToken !== undefined) return cachedToken;
  cachedToken = await invoke<string | null>("get_memory_account_token").catch(() => null);
  return cachedToken;
}

export async function saveAccountToken(token: string): Promise<void> {
  await invoke("set_memory_account_token", { token });
  cachedToken = token.trim();
}

export async function clearAccountToken(): Promise<void> {
  await invoke("clear_memory_account_token");
  cachedToken = null;
}

/** Find the agent by name on the account's roster, creating it on first use.
 *  Recall and /sleep meet on this identity. */
export async function ensureMemoryAgent(
  name: string,
  description: string,
): Promise<MemoryAgent> {
  return invoke<MemoryAgent>("ensure_memory_agent", { name, description });
}

export async function recallMemories(
  agentId: string,
  query: string,
  limit = 8,
): Promise<RecallResult> {
  return invoke<RecallResult>("recall_memories", { agentId, query, limit });
}

// --- the mind drawn (s537) ---

/** One memory as a node: the index line plus what the drawing needs. No
 *  body — `readMemory` fetches one on click. */
export interface MemoryGraphNode {
  id: string;
  name: string;
  description: string;
  mem_type: string;
  importance: number;
  access_count: number;
  project_tag: string | null;
  links: string[];
  archived_at: string | null;
  created_at: string;
  updated_at: string;
  /** Meaning-space hint (PCA-3, each axis in [-1, 1]); null when unembedded. */
  pos: [number, number, number] | null;
}

export interface MemoryGraphEdge {
  source: string;
  target: string;
  /** `link` = a [[slug]] the author wrote; `semantic` = a nearest neighbour. */
  kind: "link" | "semantic";
  /** 1.0 for a written link; cosine similarity for a semantic neighbour. */
  weight: number;
}

export interface MemoryGraphStats {
  nodes: number;
  link_edges: number;
  semantic_edges: number;
  /** [[links]] to memories never written — promises, not edges. */
  dangling_links: number;
  embedded: number;
  k: number;
  min_sim: number;
}

export interface MemoryGraph {
  nodes: MemoryGraphNode[];
  edges: MemoryGraphEdge[];
  stats: MemoryGraphStats;
}

export interface MemoryGraphOptions {
  /** Semantic neighbours per memory (0 = links only). */
  k?: number;
  /** Cosine floor for a semantic edge. */
  minSim?: number;
  includeArchived?: boolean;
}

/** The whole mind, drawn once. Read-only: walking the graph bumps no
 *  access_count — looking at a memory is not remembering it. */
export async function loadMemoryGraph(
  agentId: string,
  options: MemoryGraphOptions = {},
): Promise<MemoryGraph> {
  return invoke<MemoryGraph>("load_memory_graph", {
    agentId,
    k: options.k,
    minSim: options.minSim,
    includeArchived: options.includeArchived,
  });
}

/** One full memory by name — the graph's click-through. */
export async function readMemory(
  agentId: string,
  name: string,
): Promise<MemoryRecord> {
  return invoke<MemoryRecord>("read_memory", { agentId, name });
}

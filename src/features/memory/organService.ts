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

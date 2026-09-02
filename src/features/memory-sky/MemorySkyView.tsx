// The Memory view — a companion's mind as a sky you look into, and search.
//
// React owns the chrome (search, the hover label, the memory panel, the
// stats line); the WebGL life belongs to MemorySky. One companion's mind at a
// time: the active tab's companion by default, switchable from the HUD.

import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from "react";

import { MarkdownRenderer } from "../../components/MarkdownRenderer";
import { companionLabel, type Companion } from "../companions/types";
import { companionMemoryAgent } from "../memory/baseAgent";
import {
  loadMemoryGraph,
  readMemory,
  recallMemories,
  type MemoryGraph,
  type MemoryRecord,
} from "../memory/organService";
import { MemorySky, type SkyHit, type SkyStats } from "./engine/MemorySky";
import { typeTintCss, TYPE_ORDER } from "./palette";

interface MemorySkyViewProps {
  companions: Companion[];
  /** The conversation's companion, when the view opened from a chat. */
  initialCompanionId: string | null;
}

interface SelectedMemory {
  name: string;
  record: MemoryRecord | null;
  error: string | null;
}

export function MemorySkyView({ companions, initialCompanionId }: MemorySkyViewProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const skyRef = useRef<MemorySky | null>(null);
  const labelRef = useRef<HTMLDivElement | null>(null);

  const [companionId, setCompanionId] = useState<string | null>(initialCompanionId);
  const [agentId, setAgentId] = useState<string | null>(null);
  const [graph, setGraph] = useState<MemoryGraph | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [stats, setStats] = useState<SkyStats | null>(null);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SkyHit[] | null>(null);
  const [isSearching, setIsSearching] = useState(false);
  const [searchNote, setSearchNote] = useState<string | null>(null);
  const [selected, setSelected] = useState<SelectedMemory | null>(null);

  const companion = useMemo(
    () =>
      (companionId ? companions.find((c) => c.id === companionId) : undefined) ??
      companions.find((c) => c.isBuiltIn) ??
      companions[0] ??
      null,
    [companions, companionId],
  );

  // ── the sky itself ────────────────────────────────────────────────────
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const sky = new MemorySky(canvas, {
      onHover: (node, x, y) => {
        const label = labelRef.current;
        if (!label) return;
        if (!node) {
          label.hidden = true;
          return;
        }
        label.hidden = false;
        label.style.transform = `translate(${Math.round(x + 14)}px, ${Math.round(y - 10)}px)`;
        label.dataset.type = node.memType;
        label.textContent = node.name;
      },
      onSelect: (node) => {
        void openMemory(node?.name ?? null);
      },
      onStats: setStats,
    });
    skyRef.current = sky;
    return () => {
      sky.dispose();
      skyRef.current = null;
    };
    // openMemory is stable enough: it reads agentId through a ref below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ── load the mind ─────────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    setGraph(null);
    setLoadError(null);
    setHits(null);
    setSelected(null);
    setQuery("");
    setIsLoading(true);
    void (async () => {
      try {
        const agent = await companionMemoryAgent(companion?.id ?? null);
        if (cancelled) return;
        setAgentId(agent.agent_id);
        const loaded = await loadMemoryGraph(agent.agent_id);
        if (cancelled) return;
        setGraph(loaded);
        skyRef.current?.setGraph(loaded);
      } catch (error: unknown) {
        if (!cancelled) setLoadError(error instanceof Error ? error.message : String(error));
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [companion?.id]);

  const agentIdRef = useRef<string | null>(null);
  agentIdRef.current = agentId;

  /** Open one memory by name: the sky flies there and lights it, the panel
   *  reads it. Null closes the panel and lets the sky rest. */
  const openMemory = useCallback(async (name: string | null) => {
    const sky = skyRef.current;
    if (!name) {
      setSelected(null);
      sky?.select(null);
      return;
    }
    sky?.select(name);
    setSelected({ name, record: null, error: null });
    const agent = agentIdRef.current;
    if (!agent) return;
    try {
      const record = await readMemory(agent, name);
      setSelected((current) => (current?.name === name ? { name, record, error: null } : current));
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      setSelected((current) =>
        current?.name === name ? { name, record: null, error: message } : current,
      );
    }
  }, []);

  // ── the spell ─────────────────────────────────────────────────────────
  const castSearch = async (event: FormEvent) => {
    event.preventDefault();
    const sky = skyRef.current;
    const text = query.trim();
    if (!agentId || !sky) return;
    if (!text) {
      setHits(null);
      setSearchNote(null);
      sky.setHits([]);
      return;
    }
    setIsSearching(true);
    setSearchNote(null);
    try {
      const result = await recallMemories(agentId, text, 14);
      const found: SkyHit[] = result.hits.map((h) => ({ name: h.memory.name, score: h.score }));
      setHits(found);
      sky.setHits(found);
      if (!found.length) setSearchNote("Nothing in this mind answers that.");
      else if (!result.vector_leg) setSearchNote("Keyword leg only — the embedder is down.");
    } catch (error: unknown) {
      setSearchNote(error instanceof Error ? error.message : String(error));
    } finally {
      setIsSearching(false);
    }
  };

  const clearSearch = () => {
    setQuery("");
    setHits(null);
    setSearchNote(null);
    skyRef.current?.setHits([]);
  };

  // Esc steps back one layer at a time: the open memory closes first, then
  // the spell lifts and the sky rests. Bound on the window — the sky owns the
  // whole screen while it is up, and the exit should not depend on focus.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      if (selected) {
        event.preventDefault();
        void openMemory(null);
      } else if (hits || query) {
        event.preventDefault();
        clearSearch();
        (document.activeElement as HTMLElement | null)?.blur?.();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // clearSearch is recreated each render; the listener is rebound with it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected, hits, query, openMemory]);

  const typeCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const n of graph?.nodes ?? []) counts.set(n.mem_type, (counts.get(n.mem_type) ?? 0) + 1);
    const ordered = TYPE_ORDER.filter((t) => counts.has(t));
    for (const t of counts.keys()) if (!ordered.includes(t)) ordered.push(t);
    return ordered.map((t) => [t, counts.get(t) ?? 0] as const);
  }, [graph]);

  return (
    <div className="memory-sky">
      <canvas ref={canvasRef} className="memory-sky__canvas" />
      <div className="memory-sky__vignette" aria-hidden="true" />
      <div ref={labelRef} className="memory-sky__label" hidden />

      <div className="memory-sky__hud">
        <form className="memory-sky__search" onSubmit={castSearch}>
          <input
            type="search"
            value={query}
            placeholder={graph ? "Ask the sky…" : "Loading the mind…"}
            disabled={!graph}
            onChange={(event) => setQuery(event.target.value)}
            aria-label="Search memories"
          />
          <button type="submit" disabled={!graph || isSearching}>
            {isSearching ? "Casting…" : "Recall"}
          </button>
          {hits ? (
            <button type="button" className="memory-sky__clear" onClick={clearSearch}>
              Clear
            </button>
          ) : null}
        </form>

        {companions.length > 1 ? (
          <select
            className="memory-sky__companion"
            value={companion?.id ?? ""}
            onChange={(event) => setCompanionId(event.target.value || null)}
            aria-label="Whose mind"
          >
            {companions.map((c) => (
              <option key={c.id} value={c.id}>
                {companionLabel(c)}
              </option>
            ))}
          </select>
        ) : null}
      </div>

      {searchNote ? <p className="memory-sky__note">{searchNote}</p> : null}
      {loadError ? <p className="memory-sky__note is-error">{loadError}</p> : null}
      {isLoading ? <p className="memory-sky__note">Drawing the mind…</p> : null}

      {hits && hits.length ? (
        <ol className="memory-sky__hits">
          {hits.map((h) => (
            <li key={h.name}>
              <button type="button" onClick={() => void openMemory(h.name)}>
                <span className="memory-sky__hit-score">{Math.round(h.score * 100)}</span>
                <span className="memory-sky__hit-name">{h.name}</span>
              </button>
            </li>
          ))}
        </ol>
      ) : null}

      <footer className="memory-sky__stats">
        {graph ? (
          <>
            <span>{graph.stats.nodes.toLocaleString()} memories</span>
            <span>{graph.stats.link_edges.toLocaleString()} links</span>
            <span>{graph.stats.semantic_edges.toLocaleString()} neighbours</span>
            {graph.stats.dangling_links ? (
              <span title="[[links]] to memories never written">
                {graph.stats.dangling_links.toLocaleString()} promises
              </span>
            ) : null}
            <span className="memory-sky__legend">
              {typeCounts.map(([type, count]) => (
                <i key={type} style={{ ["--tint" as string]: typeTintCss(type) }} title={`${count} ${type}`}>
                  {type}
                </i>
              ))}
            </span>
          </>
        ) : null}
        {stats ? (
          <span className="memory-sky__fps" title="frames per second · render scale">
            {stats.fps} fps{stats.renderScale < 1 ? ` · ${Math.round(stats.renderScale * 100)}%` : ""}
            {stats.settled ? "" : " · settling"}
          </span>
        ) : null}
      </footer>

      {selected ? (
        <aside className="memory-sky__panel" aria-label="Memory">
          <header>
            <span
              className="memory-sky__panel-type"
              style={{ ["--tint" as string]: typeTintCss(selected.record?.mem_type ?? "") }}
            >
              {selected.record?.mem_type ?? "memory"}
            </span>
            <h2>{selected.name}</h2>
            <button type="button" onClick={() => void openMemory(null)} aria-label="Close">
              ×
            </button>
          </header>
          {selected.record ? (
            <>
              <p className="memory-sky__panel-desc">{selected.record.description}</p>
              <dl className="memory-sky__panel-meta">
                <div>
                  <dt>importance</dt>
                  <dd>{selected.record.importance.toFixed(2)}</dd>
                </div>
                <div>
                  <dt>recalled</dt>
                  <dd>{selected.record.access_count}×</dd>
                </div>
                <div>
                  <dt>carved</dt>
                  <dd>{formatDate(selected.record.created_at)}</dd>
                </div>
              </dl>
              <div className="memory-sky__panel-body">
                <MarkdownRenderer content={selected.record.body} />
              </div>
              {selected.record.links.length ? (
                <div className="memory-sky__panel-links">
                  {selected.record.links.map((link) => {
                    const node = graph?.nodes.find((n) => n.name === link);
                    return (
                      <button
                        key={link}
                        type="button"
                        disabled={!node}
                        title={node ? node.description : "not written yet"}
                        onClick={() => void openMemory(link)}
                      >
                        [[{link}]]
                      </button>
                    );
                  })}
                </div>
              ) : null}
            </>
          ) : selected.error ? (
            <p className="memory-sky__note is-error">{selected.error}</p>
          ) : (
            <p className="memory-sky__panel-desc">Reading…</p>
          )}
        </aside>
      ) : null}
    </div>
  );
}

function formatDate(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

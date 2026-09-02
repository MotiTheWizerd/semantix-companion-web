// The Memory view — a companion's mind as a sky you look into, and search.
//
// React owns the chrome (search, the hover label, the memory panel, the
// stats line); the WebGL life belongs to MemorySky. One companion's mind at a
// time: the active tab's companion by default, switchable from the HUD.

import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from "react";

import { MarkdownRenderer } from "../../components/MarkdownRenderer";
import { CompanionSelect } from "../companions/CompanionSelect";
import { type Companion } from "../companions/types";
import { companionMemoryAgent } from "../memory/baseAgent";
import {
  loadMemoryGraph,
  readMemory,
  recallMemories,
  type MemoryGraph,
  type MemoryRecord,
} from "../memory/organService";
import { MemorySky, type SkyFilter, type SkyHit, type SkyStats } from "./engine/MemorySky";
import { typeTintCss, TYPE_ORDER } from "./palette";

const NO_TYPES: ReadonlySet<string> = new Set();

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
  const namesRef = useRef<HTMLDivElement | null>(null);

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
  // The filter is a way of looking, not a property of one mind: it survives
  // a companion switch. Archived is a reload (the door leaves them out by
  // default); types and the floor are a lighting pass in the engine.
  const [hiddenTypes, setHiddenTypes] = useState<ReadonlySet<string>>(NO_TYPES);
  const [minImportance, setMinImportance] = useState(0);
  const [showArchived, setShowArchived] = useState(false);

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
    const sky = new MemorySky(
      canvas,
      {
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
      },
      namesRef.current,
    );
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
        const loaded = await loadMemoryGraph(agent.agent_id, { includeArchived: showArchived });
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
  }, [companion?.id, showArchived]);

  useEffect(() => {
    const filter: SkyFilter = { hiddenTypes, minImportance };
    skyRef.current?.setFilter(filter);
  }, [hiddenTypes, minImportance]);

  const toggleType = (type: string) => {
    setHiddenTypes((current) => {
      const next = new Set(current);
      if (next.has(type)) next.delete(type);
      else next.add(type);
      return next;
    });
  };

  /** Double-click a chip: only that type — or, if it already stands alone,
   *  everything back. The two clicks before it toggle twice and cancel. */
  const soloType = (type: string, allTypes: string[]) => {
    setHiddenTypes((current) => {
      const alone = !current.has(type) && allTypes.every((t) => t === type || current.has(t));
      return alone ? NO_TYPES : new Set(allTypes.filter((t) => t !== type));
    });
  };

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
      <div ref={namesRef} className="memory-sky__names" aria-hidden="true" />
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
          <CompanionSelect
            companions={companions}
            value={companion?.id ?? null}
            onChange={setCompanionId}
            variant="pill"
            eyebrow="Whose mind"
            ariaLabel="Whose mind"
            className="memory-sky__companion"
          />
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
            <span>
              {stats && stats.visible < graph.stats.nodes
                ? `${stats.visible.toLocaleString()} of ${graph.stats.nodes.toLocaleString()} memories`
                : `${graph.stats.nodes.toLocaleString()} memories`}
            </span>
            <span>{graph.stats.link_edges.toLocaleString()} links</span>
            <span>{graph.stats.semantic_edges.toLocaleString()} neighbours</span>
            {graph.stats.dangling_links ? (
              <span title="[[links]] to memories never written">
                {graph.stats.dangling_links.toLocaleString()} promises
              </span>
            ) : null}
            <span className="memory-sky__legend" role="group" aria-label="Show types">
              {typeCounts.map(([type, count]) => (
                <button
                  key={type}
                  type="button"
                  className="memory-sky__chip"
                  style={{ ["--tint" as string]: typeTintCss(type) }}
                  aria-pressed={!hiddenTypes.has(type)}
                  title={`${count} ${type} · click to toggle · double-click for only this`}
                  onClick={() => toggleType(type)}
                  onDoubleClick={() => soloType(type, typeCounts.map(([t]) => t))}
                >
                  {type}
                </button>
              ))}
              {hiddenTypes.size ? (
                <button
                  type="button"
                  className="memory-sky__chip is-reset"
                  onClick={() => setHiddenTypes(NO_TYPES)}
                  title="Show every type"
                >
                  all
                </button>
              ) : null}
            </span>
            <label className="memory-sky__floor" title="Sink memories below this importance">
              <span>≥ {minImportance.toFixed(2)}</span>
              <input
                type="range"
                min={0}
                max={0.95}
                step={0.05}
                value={minImportance}
                onChange={(event) => setMinImportance(Number(event.target.value))}
                aria-label="Importance floor"
              />
            </label>
            <label className="memory-sky__archived" title="Archived memories, drawn as ghosts">
              <input
                type="checkbox"
                checked={showArchived}
                onChange={(event) => setShowArchived(event.target.checked)}
              />
              archived
            </label>
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

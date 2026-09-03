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
  loadMemoryNodes,
  readMemory,
  recallMemories,
  type MemoryGraph,
  type MemoryRecord,
} from "../memory/organService";
import { onMemorySlept } from "../memory/sleepService";
import { edgeKey, MemorySky, type SkyFilter, type SkyHit, type SkyStats } from "./engine/MemorySky";
import { typeTintCss, TYPE_ORDER } from "./palette";

const NO_TYPES: ReadonlySet<string> = new Set();

/** THE LIVING SKY (s545). The sleeper carves while the app is used, so the
 *  mind on screen is only ever a photograph unless something tells it. Two
 *  things do:
 *
 *  · a pass that lands WHILE the sky is up grows it — the newborn is appended
 *    and struck, and the layout makes room;
 *  · a pass that landed while it was NOT up arrives inside the graph itself,
 *    so the memories carved since this mind was last looked at are struck on
 *    arrival. Without this the beautiful moment would need him to be on the
 *    right screen at the right second, which is almost never.
 *
 *  The watermark is the newest `created_at` already shown, per mind, for this
 *  run of the app. A mind's FIRST look strikes nothing — 2,400 memories are
 *  not news — it just sets the mark. */
const SEEN_UNTIL = new Map<string, number>();
/** A long absence is still one arrival, not a light show. */
const MAX_REPLAY_STRIKES = 24;
/** How long the sky says what it just learned. */
const BIRTH_NOTE_MS = 7000;

/** Fold a delta from `/graph/nodes` into the loaded graph, so the footer's
 *  counts and the type chips stay true to what is on screen. `dangling_links`
 *  adds: a promise a newborn makes is new, and one it FULFILS is only
 *  subtracted on the next full draw. */
function mergeDelta(base: MemoryGraph, delta: MemoryGraph): MemoryGraph {
  const nodes = base.nodes.slice();
  const at = new Map(base.nodes.map((node, index) => [node.name, index]));
  for (const node of delta.nodes) {
    const index = at.get(node.name);
    if (index === undefined) {
      at.set(node.name, nodes.length);
      nodes.push(node);
    } else {
      nodes[index] = node;
    }
  }
  const edges = base.edges.slice();
  const drawn = new Set(base.edges.map((e) => edgeKey(e.kind, e.source, e.target)));
  for (const edge of delta.edges) {
    const key = edgeKey(edge.kind, edge.source, edge.target);
    if (drawn.has(key)) continue;
    drawn.add(key);
    edges.push(edge);
  }
  return {
    nodes,
    edges,
    stats: {
      ...base.stats,
      nodes: nodes.length,
      link_edges: edges.reduce((n, e) => n + (e.kind === "link" ? 1 : 0), 0),
      semantic_edges: edges.reduce((n, e) => n + (e.kind === "semantic" ? 1 : 0), 0),
      dangling_links: base.stats.dangling_links + delta.stats.dangling_links,
    },
  };
}

/** Memories in this graph carved after `since` — oldest first, capped. */
function bornSince(graph: MemoryGraph, since: number): string[] {
  return graph.nodes
    .filter((node) => Date.parse(node.created_at) > since)
    .sort((a, b) => Date.parse(a.created_at) - Date.parse(b.created_at))
    .slice(-MAX_REPLAY_STRIKES)
    .map((node) => node.name);
}

function newestCarve(graph: MemoryGraph): number {
  let newest = 0;
  for (const node of graph.nodes) {
    const at = Date.parse(node.created_at);
    if (at > newest) newest = at;
  }
  return newest;
}

/** What the sky just learned, said in the chrome. One memory gets its NAME —
 *  a birth on a 2,459-memory mind is unreadable otherwise, and the name is
 *  also the way back to it after the hold lets go. */
function bornNote(names: string[]): { text: string; open: string | null } {
  if (names.length === 1) return { text: `Just carved · ${names[0]}`, open: names[0] };
  return { text: `${names.length} memories were just carved.`, open: null };
}

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
  /** What the mind just learned, said for a few seconds under the search. */
  const [birthNote, setBirthNote] = useState<{ text: string; open: string | null } | null>(null);

  const agentIdRef = useRef<string | null>(null);
  agentIdRef.current = agentId;
  const showArchivedRef = useRef(showArchived);
  showArchivedRef.current = showArchived;
  const birthTimerRef = useRef<number | null>(null);

  const announceBirth = useCallback((names: string[]) => {
    if (!names.length) return;
    setBirthNote(bornNote(names));
    if (birthTimerRef.current) window.clearTimeout(birthTimerRef.current);
    birthTimerRef.current = window.setTimeout(() => setBirthNote(null), BIRTH_NOTE_MS);
  }, []);
  // The engine is built once and outlives every render; it reaches the current
  // announcer through this rather than being rebuilt to capture a new one.
  const announceBirthRef = useRef(announceBirth);
  announceBirthRef.current = announceBirth;

  useEffect(
    () => () => {
      if (birthTimerRef.current) window.clearTimeout(birthTimerRef.current);
    },
    [],
  );

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
        onStruck: (names) => announceBirthRef.current?.(names),
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
        // What this mind learned since it was last looked at, struck on
        // arrival. First look sets the mark and stays calm.
        const since = SEEN_UNTIL.get(agent.agent_id);
        SEEN_UNTIL.set(agent.agent_id, newestCarve(loaded));
        // The strike waits for the layout to rest, and announces itself then.
        if (since !== undefined) skyRef.current?.strike(bornSince(loaded, since));
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

  // ── the living sky ────────────────────────────────────────────────────
  // A pass that lands while this view is up grows the mind on screen: fetch
  // only what was carved and its edges, append, strike. The whole graph is
  // never refetched — that is 3.6MB and a lost layout on a big mind.
  useEffect(() => {
    if (!agentId) return;
    let stopped = false;
    let unlisten: (() => void) | undefined;
    void onMemorySlept((event) => {
      if (stopped || event.kind !== "carved") return;
      if (event.agentId !== agentIdRef.current || !event.memories.length) return;
      void (async () => {
        const sky = skyRef.current;
        const agent = agentIdRef.current;
        if (!sky || !agent) return;
        try {
          const delta = await loadMemoryNodes(agent, event.memories, {
            includeArchived: showArchivedRef.current,
          });
          if (stopped || !delta.nodes.length) return;
          sky.addMemories(delta); // strikes, and announces through onStruck
          setGraph((current) => (current ? mergeDelta(current, delta) : current));
          SEEN_UNTIL.set(agent, Math.max(SEEN_UNTIL.get(agent) ?? 0, newestCarve(delta)));
        } catch {
          // The sky simply does not grow this time; the next full draw has it.
        }
      })();
    }).then((fn) => {
      if (stopped) fn();
      else unlisten = fn;
    });
    return () => {
      stopped = true;
      unlisten?.();
    };
  }, [agentId]);

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
      } else if (skyRef.current?.rest()) {
        // A birth was holding the sky; Esc lets it go early.
        event.preventDefault();
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

      {birthNote ? (
        birthNote.open ? (
          <button
            type="button"
            className="memory-sky__note is-born"
            title="Open this memory"
            onClick={() => void openMemory(birthNote.open)}
          >
            {birthNote.text}
          </button>
        ) : (
          <p className="memory-sky__note is-born">{birthNote.text}</p>
        )
      ) : null}
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

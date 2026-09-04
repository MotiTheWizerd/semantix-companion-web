// The legend — the sidebar's face while the sky is up. Where the chat keeps
// its recent conversations, the sky keeps its key: every memory type as a
// glowing row you can put out, the importance floor, the archived ghosts,
// and the mind's own numbers. It reads the lens store; the sky writes it.
//
// On the chat's bundle: nothing here may import the engine or palette.ts.

import { useShallow } from "zustand/react/shallow";

import { useSkyLensStore } from "./skyLensStore";
import { TYPE_ORDER, typeTintCss } from "./tints";

export function SkyLegend() {
  const {
    hiddenTypes,
    minImportance,
    showArchived,
    typeCounts,
    mind,
    sky,
    toggleType,
    soloType,
    showAllTypes,
    setMinImportance,
    setShowArchived,
  } = useSkyLensStore(
    useShallow((state) => ({
      hiddenTypes: state.hiddenTypes,
      minImportance: state.minImportance,
      showArchived: state.showArchived,
      typeCounts: state.typeCounts,
      mind: state.mind,
      sky: state.sky,
      toggleType: state.toggleType,
      soloType: state.soloType,
      showAllTypes: state.showAllTypes,
      setMinImportance: state.setMinImportance,
      setShowArchived: state.setShowArchived,
    })),
  );

  // Before a mind is loaded the key still stands, quietly: the known types
  // with no counts, so the pane never flashes empty on a companion switch.
  const rows: ReadonlyArray<readonly [string, number | null]> = typeCounts.length
    ? typeCounts
    : TYPE_ORDER.map((type) => [type, null] as const);
  const largest = rows.reduce((max, [, count]) => Math.max(max, count ?? 0), 0);

  const visible = sky && mind && sky.visible < mind.nodes ? sky.visible : null;

  return (
    <div className="sky-legend" aria-label="Legend">
      <p className="sidebar-section-label">Legend</p>

      <div className="sky-legend__types" role="group" aria-label="Show types">
        {rows.map(([type, count]) => (
          <button
            key={type}
            type="button"
            className="sky-legend__type"
            style={{
              ["--tint" as string]: typeTintCss(type),
              ["--share" as string]: largest ? (count ?? 0) / largest : 0,
            }}
            aria-pressed={!hiddenTypes.has(type)}
            title={
              count === null
                ? type
                : `${count} ${type} · click to toggle · double-click for only this`
            }
            onClick={() => toggleType(type)}
            onDoubleClick={() => soloType(type)}
          >
            <span className="sky-legend__dot" aria-hidden="true" />
            <span className="sky-legend__name">{type}</span>
            <span className="sky-legend__count">{count === null ? "" : count.toLocaleString()}</span>
            <span className="sky-legend__bar" aria-hidden="true" />
          </button>
        ))}
      </div>

      {hiddenTypes.size ? (
        <button type="button" className="sky-legend__all" onClick={showAllTypes}>
          Light every type
        </button>
      ) : null}

      <div className="sky-legend__lens">
        <label className="sky-legend__floor" title="Sink memories below this importance">
          <span className="sky-legend__lens-label">
            Importance
            <b>≥ {minImportance.toFixed(2)}</b>
          </span>
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

        <label className="sky-legend__archived" title="Archived memories, drawn as ghosts">
          <input
            type="checkbox"
            checked={showArchived}
            onChange={(event) => setShowArchived(event.target.checked)}
          />
          <span>Archived, as ghosts</span>
        </label>
      </div>

      <dl className="sky-legend__mind">
        <div>
          <dd>
            {visible !== null ? (
              <>
                {visible.toLocaleString()}
                <small> of {mind?.nodes.toLocaleString()}</small>
              </>
            ) : (
              (mind?.nodes.toLocaleString() ?? "—")
            )}
          </dd>
          <dt>memories</dt>
        </div>
        <div>
          <dd>{mind?.link_edges.toLocaleString() ?? "—"}</dd>
          <dt>links</dt>
        </div>
        <div>
          <dd>{mind?.semantic_edges.toLocaleString() ?? "—"}</dd>
          <dt>neighbours</dt>
        </div>
        {mind?.dangling_links ? (
          <div title="[[links]] to memories never written">
            <dd>{mind.dangling_links.toLocaleString()}</dd>
            <dt>promises</dt>
          </div>
        ) : null}
      </dl>

      {sky ? (
        <p className="sky-legend__pulse" title="frames per second · render scale">
          {sky.fps} fps{sky.renderScale < 1 ? ` · ${Math.round(sky.renderScale * 100)}%` : ""}
          {sky.settled ? "" : " · settling"}
        </p>
      ) : null}
    </div>
  );
}

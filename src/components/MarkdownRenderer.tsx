// Streaming markdown renderer — ported from the studio (s348, proven
// byte-identical to a one-shot render). Uses our own converter in
// src/lib/markdown; the only external dependency is highlight.js.

import { useRef } from "react";
import { toHtml } from "../lib/markdown/index";

interface MarkdownRendererProps {
  content: string;
}

/**
 * Incremental conversion state for a streaming message. A growing message
 * re-rendered per delta flush would re-parse its WHOLE markdown every time
 * (O(n²) over the stream); instead we freeze everything up to the last "safe
 * cut" and only re-parse the still-streaming tail.
 *
 * A safe cut is a blank line ("\n\n") outside a code fence. Our block parsers
 * all consume contiguous non-blank runs (paragraph/list/table/quote stop at a
 * blank line; heading/hr are single-line), and parseBlocks joins block html
 * with '\n' — so toHtml(head) + '\n' + toHtml(tail) at such a cut is EXACTLY
 * toHtml(head + tail), not an approximation. Fences are the one construct
 * that may span blank lines; cuts inside one are rejected by ``` parity.
 */
interface IncrementalState {
  /** Full source last converted. */
  src: string;
  /** Chars of src whose html is settled (ends just after a safe cut). */
  stableLen: number;
  /** Settled html, one APPEND-ONLY segment per settle event. Each segment
   *  becomes its own DOM node whose innerHTML never changes again — so a
   *  streamed delta re-writes ONLY the open tail's node instead of nuking
   *  and rebuilding the whole transcript subtree (layout+paint per flush,
   *  brutal on WebKitGTK/llvmpipe). */
  blocks: string[];
  /** Html of the still-open tail (re-converted per flush). */
  tailHtml: string;
}

/** Last safe-cut offset in `region` (position just after a "\n\n" with an even
 *  number of ``` markers before it), or -1. One forward pass, each fence
 *  marker counted once. */
function lastSafeCut(region: string): number {
  let cut = -1;
  let parity = 0;
  let scanned = 0;
  let p = region.indexOf("\n\n");
  while (p !== -1) {
    let f = region.indexOf("```", scanned);
    while (f !== -1 && f < p) {
      parity ^= 1;
      f = region.indexOf("```", f + 3);
    }
    scanned = p;
    if (parity === 0) cut = p + 2;
    p = region.indexOf("\n\n", p + 1);
  }
  return cut;
}

const EMPTY: IncrementalState = { src: "", stableLen: 0, blocks: [], tailHtml: "" };

function useIncrementalHtml(content: string): IncrementalState {
  const stateRef = useRef<IncrementalState | null>(null);

  if (!content) {
    stateRef.current = null;
    return EMPTY;
  }

  const prev = stateRef.current;
  if (prev && prev.src === content) return prev;

  let stableLen: number;
  let blocks: string[];
  if (prev && content.length > prev.src.length && content.startsWith(prev.src)) {
    // The streaming path: pure append. Settled segments keep their html
    // (and their identity — React never re-writes those nodes).
    stableLen = prev.stableLen;
    blocks = prev.blocks;
  } else {
    // New/edited content — start over.
    stableLen = 0;
    blocks = [];
  }

  // Advance the stable cut past any newly-completed blocks, then convert only
  // what's still open. The parity scan starts at stableLen, which by
  // construction sits at even fence parity.
  const region = content.slice(stableLen);
  const cut = lastSafeCut(region);
  if (cut > 0) {
    blocks = [...blocks, toHtml(region.slice(0, cut))];
    stableLen += cut;
  }
  const tailHtml = toHtml(content.slice(stableLen));
  const next = { src: content, stableLen, blocks, tailHtml };
  stateRef.current = next;
  return next;
}

/**
 * Renders markdown content as HTML with syntax highlighting. Streaming-aware
 * twice over: settled blocks are CONVERTED once (only the open tail re-parses
 * per delta flush), and they're also RENDERED once — each settled segment is
 * its own `.md-seg` node whose innerHTML never changes, so a delta flush
 * re-writes only the tail node. `.md-seg` is display:contents (markdown.css)
 * — zero layout impact.
 */
export function MarkdownRenderer({ content }: MarkdownRendererProps) {
  const { blocks, tailHtml } = useIncrementalHtml(content);

  return (
    <div className="md-content">
      {blocks.map((h, i) => (
        <div key={i} className="md-seg" dangerouslySetInnerHTML={{ __html: h }} />
      ))}
      {tailHtml && <div className="md-seg" dangerouslySetInnerHTML={{ __html: tailHtml }} />}
    </div>
  );
}

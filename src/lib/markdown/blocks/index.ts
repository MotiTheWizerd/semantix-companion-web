import type { BlockParser } from './types'
import { PLACEHOLDER_PREFIX } from '../highlighter'
import { horizontalRuleParser } from './horizontal-rule'
import { headingParser } from './heading'
import { blockquoteParser } from './blockquote'
import { unorderedListParser, orderedListParser } from './list'
import { tableParser } from './table'
import { paragraphParser } from './paragraph'

// Order matters: more specific parsers first, paragraph is the fallback
const blockParsers: BlockParser[] = [
  horizontalRuleParser,
  headingParser,
  blockquoteParser,
  unorderedListParser,
  orderedListParser,
  tableParser,
  paragraphParser,
]

export function parseBlocks(text: string): string {
  const lines = text.split('\n')
  const output: string[] = []
  let i = 0

  while (i < lines.length) {
    const line = lines[i]

    // Code block placeholder — pass through
    if (line.includes(PLACEHOLDER_PREFIX)) {
      output.push(line)
      i++
      continue
    }

    // Blank line — skip
    if (line.trim() === '') {
      i++
      continue
    }

    // Try each block parser
    const ctx = { lines, index: i }
    const parser = blockParsers.find(p => p.match(line, ctx))
    if (parser) {
      const result = parser.parse(ctx)
      if (result.html) output.push(result.html)
      i = result.index
    } else {
      i++
    }
  }

  return output.join('\n')
}

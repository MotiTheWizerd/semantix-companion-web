import type { BlockParser, BlockParseContext, BlockParseResult } from './types'
import { parseInline } from '../inline'
import { PLACEHOLDER_PREFIX } from '../highlighter'

const BLOCK_BREAK_RE = /^#{1,3}\s|^> |^(\s*)- |^\d+\.\s|^---+$/

export const paragraphParser: BlockParser = {
  match(line: string): boolean {
    return line.trim() !== ''
  },

  parse(ctx: BlockParseContext): BlockParseResult {
    const paraLines: string[] = []
    let i = ctx.index
    while (i < ctx.lines.length) {
      const cur = ctx.lines[i]
      if (cur.trim() === '' || BLOCK_BREAK_RE.test(cur) || cur.includes(PLACEHOLDER_PREFIX) || cur.includes('|')) {
        break
      }
      paraLines.push(cur)
      i++
    }
    if (paraLines.length === 0) {
      return { html: '', index: i + 1 }
    }
    return {
      html: `<p>${paraLines.map(l => parseInline(l)).join('<br>')}</p>`,
      index: i,
    }
  },
}

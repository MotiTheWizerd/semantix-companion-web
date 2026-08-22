import type { BlockParser, BlockParseContext, BlockParseResult } from './types'
import { parseInline } from '../inline'

export const headingParser: BlockParser = {
  match(line: string): boolean {
    return /^#{1,3}\s+.+$/.test(line)
  },

  parse(ctx: BlockParseContext): BlockParseResult {
    const match = ctx.lines[ctx.index].match(/^(#{1,3})\s+(.+)$/)!
    const level = match[1].length
    return {
      html: `<h${level} class="md-h${level}">${parseInline(match[2])}</h${level}>`,
      index: ctx.index + 1,
    }
  },
}

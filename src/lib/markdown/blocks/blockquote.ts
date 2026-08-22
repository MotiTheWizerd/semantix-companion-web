import type { BlockParser, BlockParseContext, BlockParseResult } from './types'
import { parseInline } from '../inline'

export const blockquoteParser: BlockParser = {
  match(line: string): boolean {
    return line.startsWith('> ')
  },

  parse(ctx: BlockParseContext): BlockParseResult {
    const quoteLines: string[] = []
    let i = ctx.index
    while (i < ctx.lines.length && ctx.lines[i].startsWith('> ')) {
      quoteLines.push(ctx.lines[i].slice(2))
      i++
    }
    return {
      html: `<blockquote class="md-blockquote">${quoteLines.map(l => parseInline(l)).join('<br>')}</blockquote>`,
      index: i,
    }
  },
}

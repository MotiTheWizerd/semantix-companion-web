import type { BlockParser, BlockParseContext, BlockParseResult } from './types'

export const horizontalRuleParser: BlockParser = {
  match(line: string): boolean {
    return /^---+$/.test(line.trim())
  },

  parse(ctx: BlockParseContext): BlockParseResult {
    return {
      html: '<hr class="md-hr">',
      index: ctx.index + 1,
    }
  },
}

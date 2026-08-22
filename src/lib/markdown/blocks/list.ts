import type { BlockParser, BlockParseContext, BlockParseResult } from './types'
import { parseInline } from '../inline'

export const unorderedListParser: BlockParser = {
  match(line: string): boolean {
    return /^(\s*)- /.test(line)
  },

  parse(ctx: BlockParseContext): BlockParseResult {
    const listItems: string[] = []
    let i = ctx.index
    while (i < ctx.lines.length && /^(\s*)- /.test(ctx.lines[i])) {
      const itemMatch = ctx.lines[i].match(/^(\s*)- (.+)$/)
      if (itemMatch) {
        const indent = itemMatch[1].length
        const content = parseInline(itemMatch[2])
        if (indent >= 2) {
          const last = listItems.length - 1
          if (last >= 0) {
            listItems[last] += `<ul class="md-ul"><li>${content}</li></ul>`
          }
        } else {
          listItems.push(`<li>${content}</li>`)
        }
      }
      i++
    }
    return {
      html: `<ul class="md-ul">${listItems.join('')}</ul>`,
      index: i,
    }
  },
}

export const orderedListParser: BlockParser = {
  match(line: string): boolean {
    return /^\d+\.\s/.test(line)
  },

  parse(ctx: BlockParseContext): BlockParseResult {
    const listItems: string[] = []
    let i = ctx.index
    while (i < ctx.lines.length && /^\d+\.\s/.test(ctx.lines[i])) {
      const itemMatch = ctx.lines[i].match(/^\d+\.\s(.+)$/)
      if (itemMatch) {
        listItems.push(`<li>${parseInline(itemMatch[1])}</li>`)
      }
      i++
    }
    return {
      html: `<ol class="md-ol">${listItems.join('')}</ol>`,
      index: i,
    }
  },
}

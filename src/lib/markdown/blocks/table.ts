import type { BlockParser, BlockParseContext, BlockParseResult } from './types'
import { parseInline } from '../inline'

function parseRow(row: string): string[] {
  return row.replace(/^\|/, '').replace(/\|$/, '').split('|').map(c => c.trim())
}

function parseAlignments(separatorRow: string): string[] {
  return parseRow(separatorRow).map(c => {
    if (c.startsWith(':') && c.endsWith(':')) return 'center'
    if (c.endsWith(':')) return 'right'
    return 'left'
  })
}

const SEPARATOR_RE = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/

export const tableParser: BlockParser = {
  match(line: string, ctx: BlockParseContext): boolean {
    return line.includes('|') &&
      ctx.index + 1 < ctx.lines.length &&
      SEPARATOR_RE.test(ctx.lines[ctx.index + 1])
  },

  parse(ctx: BlockParseContext): BlockParseResult {
    const headers = parseRow(ctx.lines[ctx.index])
    const aligns = parseAlignments(ctx.lines[ctx.index + 1])
    let i = ctx.index + 2 // skip header + separator

    const headHtml = headers.map((h, j) =>
      `<th style="text-align:${aligns[j] ?? 'left'}">${parseInline(h)}</th>`
    ).join('')

    const bodyRows: string[] = []
    while (i < ctx.lines.length && ctx.lines[i].includes('|')) {
      const cells = parseRow(ctx.lines[i])
      bodyRows.push('<tr>' + cells.map((c, j) =>
        `<td style="text-align:${aligns[j] ?? 'left'}">${parseInline(c)}</td>`
      ).join('') + '</tr>')
      i++
    }

    return {
      html: `<div class="mb-[1.5rem]"><table class="md-table"><thead><tr>${headHtml}</tr></thead><tbody>${bodyRows.join('')}</tbody></table></div>`,
      index: i,
    }
  },
}

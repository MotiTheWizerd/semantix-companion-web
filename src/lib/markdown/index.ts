import { extractCodeBlocks, restorePlaceholders } from './highlighter'
import { parseBlocks } from './blocks'

export function toHtml(markdown: string): string {
  if (!markdown) return ''
  const { text, blocks } = extractCodeBlocks(markdown)
  const html = parseBlocks(text)
  return restorePlaceholders(html, blocks)
}

// Re-export utilities for advanced usage
export { extractCodeBlocks, restorePlaceholders } from './highlighter'
export { parseBlocks } from './blocks'
export { parseInline, escapeHtml } from './inline'

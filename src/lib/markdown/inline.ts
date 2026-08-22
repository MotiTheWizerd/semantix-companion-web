export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

export function parseInline(line: string): string {
  let result = escapeHtml(line)
  // Bold: **text**
  result = result.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
  // Italic: *text*
  result = result.replace(/\*(.+?)\*/g, '<em>$1</em>')
  // Inline code: `text`
  result = result.replace(/`([^`]+)`/g, '<code class="md-inline-code">$1</code>')
  // Links: [text](url)
  result = result.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>')
  return result
}

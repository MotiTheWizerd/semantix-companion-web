import hljs from 'highlight.js/lib/core'
import javascript from 'highlight.js/lib/languages/javascript'
import typescript from 'highlight.js/lib/languages/typescript'
import python from 'highlight.js/lib/languages/python'
import rust from 'highlight.js/lib/languages/rust'
import bash from 'highlight.js/lib/languages/bash'
import json from 'highlight.js/lib/languages/json'
import css from 'highlight.js/lib/languages/css'
import xml from 'highlight.js/lib/languages/xml'
import sql from 'highlight.js/lib/languages/sql'
import yaml from 'highlight.js/lib/languages/yaml'
import markdown from 'highlight.js/lib/languages/markdown'
import java from 'highlight.js/lib/languages/java'
import cpp from 'highlight.js/lib/languages/cpp'
import go from 'highlight.js/lib/languages/go'
import csharp from 'highlight.js/lib/languages/csharp'
import { escapeHtml } from './inline'

hljs.registerLanguage('javascript', javascript)
hljs.registerLanguage('js', javascript)
hljs.registerLanguage('jsx', javascript)
hljs.registerLanguage('typescript', typescript)
hljs.registerLanguage('ts', typescript)
hljs.registerLanguage('tsx', typescript)
hljs.registerLanguage('python', python)
hljs.registerLanguage('py', python)
hljs.registerLanguage('rust', rust)
hljs.registerLanguage('bash', bash)
hljs.registerLanguage('sh', bash)
hljs.registerLanguage('json', json)
hljs.registerLanguage('css', css)
hljs.registerLanguage('html', xml)
hljs.registerLanguage('xml', xml)
hljs.registerLanguage('sql', sql)
hljs.registerLanguage('yaml', yaml)
hljs.registerLanguage('yml', yaml)
hljs.registerLanguage('markdown', markdown)
hljs.registerLanguage('md', markdown)
hljs.registerLanguage('java', java)
hljs.registerLanguage('cpp', cpp)
hljs.registerLanguage('c', cpp)
hljs.registerLanguage('go', go)
hljs.registerLanguage('csharp', csharp)
hljs.registerLanguage('cs', csharp)

export const PLACEHOLDER_PREFIX = '\x00CODE_'

// Highlight cache: a streaming message re-runs extractCodeBlocks over its FULL
// text on every delta flush, so every already-settled block would re-highlight
// each time (hljs is the expensive pass — highlightAuto tries every registered
// grammar). Block content is stable once its closing fence arrives, so cache
// by lang+code. Bounded by wholesale clear: refill costs one frame's blocks.
const HIGHLIGHT_CACHE_MAX = 500
const highlightCache = new Map<string, string>()

function highlightBlock(lang: string, code: string, cache = true): string {
  const key = `${lang}\x00${code}`
  const cached = highlightCache.get(key)
  if (cached !== undefined) return cached
  const highlighted =
    lang && hljs.getLanguage(lang)
      ? hljs.highlight(code, { language: lang }).value
      : hljs.highlightAuto(code).value
  if (cache) {
    if (highlightCache.size >= HIGHLIGHT_CACHE_MAX) highlightCache.clear()
    highlightCache.set(key, highlighted)
  }
  return highlighted
}

function blockHtml(lang: string, code: string, cache: boolean): string {
  const highlighted = highlightBlock(lang, code.replace(/\n$/, ''), cache)
  const langClass = lang ? ` class="language-${escapeHtml(lang)} hljs"` : ' class="hljs"'
  const langLabel = lang ? `<span class="md-code-lang">${escapeHtml(lang)}</span>` : ''
  return `<div class="md-code-block m">${langLabel}<pre><code${langClass}>${highlighted}</code></pre></div>`
}

export function extractCodeBlocks(text: string): { text: string; blocks: Map<string, string> } {
  const blocks = new Map<string, string>()
  let idx = 0
  let result = text.replace(/```(\w*)\n([\s\S]*?)```/g, (_match, lang: string, code: string) => {
    const key = `${PLACEHOLDER_PREFIX}${idx++}\x00`
    blocks.set(key, blockHtml(lang, code, true))
    return key
  })
  // Trailing OPEN fence — a streaming block whose closer hasn't arrived yet.
  // Without this it falls through to the paragraph parser (visible backticks,
  // newlines collapsed) until the closing fence lands. CommonMark says an
  // unclosed fence runs to end-of-input, so render it as a code block and
  // highlight what's here — the block then colors itself as it streams.
  // Uncached: the code grows every flush, so each snapshot is seen once.
  const open = result.lastIndexOf('```')
  if (open !== -1) {
    const m = /^```(\w*)(?:\n([\s\S]*))?$/.exec(result.slice(open))
    if (m) {
      const key = `${PLACEHOLDER_PREFIX}${idx++}\x00`
      blocks.set(key, blockHtml(m[1], m[2] ?? '', false))
      result = result.slice(0, open) + key
    }
  }
  return { text: result, blocks }
}

export function restorePlaceholders(html: string, blocks: Map<string, string>): string {
  let result = html
  for (const [key, value] of blocks) {
    result = result.replace(key, value)
  }
  return result
}

export interface BlockParseContext {
  lines: string[]
  index: number
}

export interface BlockParseResult {
  html: string
  index: number
}

export interface BlockParser {
  match(line: string, ctx: BlockParseContext): boolean
  parse(ctx: BlockParseContext): BlockParseResult
}

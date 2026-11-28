import type { MarkdownNode } from '#components/markdown-message-types'
import type { MarkdownParser } from '#components/remark-markdown-details/source'

export type SourceOffsetMap = readonly number[]
export type MarkdownSource = {
  source: string
  lineAtOffset: Uint32Array
  lineStarts: number[]
}
export type HtmlLexeme = { at: number; value: string }
export type MarkdownCodeRange = {
  end: number
  start: number
  type: 'code' | 'inlineCode'
}
export type HtmlSourceGroup = {
  node: MarkdownNode
  offset: number
  offsets: SourceOffsetMap
}
export type HtmlSourceAnalysis = {
  codeRanges: MarkdownCodeRange[]
  group: HtmlSourceGroup
  lexemes: HtmlLexeme[]
  maskedSource: string
}

export function createMarkdownSource(source: string): MarkdownSource {
  const lineAtOffset = new Uint32Array(source.length + 1)
  const lineStarts = [0]
  let line = 0
  for (let offset = 0; offset < source.length; offset += 1) {
    lineAtOffset[offset] = line
    if (source.charCodeAt(offset) === 10) {
      line += 1
      lineStarts.push(offset + 1)
    }
  }
  lineAtOffset[source.length] = line
  return { source, lineAtOffset, lineStarts }
}

function sourceLine(markdown: MarkdownSource, line: number) {
  const start = markdown.lineStarts[line] ?? markdown.source.length
  const next = markdown.lineStarts[line + 1] ?? markdown.source.length
  const lineFeed = next > start && markdown.source.charCodeAt(next - 1) === 10
  const carriageReturn =
    lineFeed && next - 2 >= start && markdown.source.charCodeAt(next - 2) === 13
  const end = next - (lineFeed ? (carriageReturn ? 2 : 1) : 0)
  return { end, start, value: markdown.source.slice(start, end) }
}

export function createHtmlSourceGroup(
  node: MarkdownNode,
  markdown: MarkdownSource,
): HtmlSourceGroup {
  const value = node.value ?? ''
  const base = node.position?.start.offset ?? 0
  const offsets = Array<number>(value.length + 1).fill(base)
  let localStart = 0
  let sourceLineIndex =
    markdown.lineAtOffset[Math.min(base, markdown.source.length)] ?? 0
  let firstLine = true

  while (localStart <= value.length) {
    const newline = value.indexOf('\n', localStart)
    const localEnd = newline === -1 ? value.length : newline
    const rawLine = value.slice(localStart, localEnd)
    const line = rawLine.endsWith('\r') ? rawLine.slice(0, -1) : rawLine
    const originalLine = sourceLine(markdown, sourceLineIndex)
    const from = firstLine ? Math.max(0, base - originalLine.start) : 0
    let withinLine = originalLine.value.indexOf(line, from)
    if (withinLine === -1) withinLine = originalLine.value.indexOf(line)
    if (withinLine === -1) withinLine = from
    const globalStart =
      line.length === 0 ? originalLine.end : originalLine.start + withinLine

    for (let index = 0; index <= line.length; index += 1) {
      offsets[localStart + index] = globalStart + index
    }
    if (rawLine.length > line.length) {
      offsets[localEnd] = originalLine.end
    }
    if (newline === -1) break

    offsets[localEnd] = globalStart + line.length
    localStart = localEnd + 1
    sourceLineIndex = Math.min(
      sourceLineIndex + 1,
      markdown.lineStarts.length - 1,
    )
    firstLine = false
  }

  return { node, offset: offsets[0] ?? base, offsets }
}

function maskHtmlLexemes(source: string, lexemes: readonly HtmlLexeme[]) {
  const parts: string[] = []
  let cursor = 0
  for (const lexeme of lexemes) {
    parts.push(source.slice(cursor, lexeme.at))
    parts.push(lexeme.value.replace(/[^\r\n]/g, 'x'))
    cursor = lexeme.at + lexeme.value.length
  }
  parts.push(source.slice(cursor))
  return parts.join('')
}

function collectMarkdownCodeRanges(nodes: readonly MarkdownNode[]) {
  const ranges: MarkdownCodeRange[] = []
  const pending = [...nodes].reverse()
  while (pending.length > 0) {
    const node = pending.pop()
    if (node === undefined) continue
    if (node.type === 'code' || node.type === 'inlineCode') {
      const start = node.position?.start.offset
      const end = node.position?.end.offset
      if (start !== undefined && end !== undefined) {
        ranges.push({ end, start, type: node.type })
      }
    }
    if (node.children !== undefined) {
      for (let index = node.children.length - 1; index >= 0; index -= 1) {
        const child = node.children[index]
        if (child !== undefined) pending.push(child)
      }
    }
  }
  return ranges
}

function rangeContains(range: MarkdownCodeRange, offset: number) {
  return range.start <= offset && offset < range.end
}

function getCommonMarkHtmlInterruptions(
  source: string,
  lexemes: readonly HtmlLexeme[],
  fencedRanges: readonly MarkdownCodeRange[],
) {
  const interruptions: number[] = []
  const lexemeStarts = new Set(lexemes.map((lexeme) => lexeme.at))
  let fenceIndex = 0
  const standaloneTags = /^[ \t]{0,3}(<\/?(?:details|summary)\b[^>\r\n]*>)/gim
  for (const match of source.matchAll(standaloneTags)) {
    const lineStart = match.index
    const at = lineStart + match[0].indexOf(match[1] ?? '')
    if (!lexemeStarts.has(at)) continue
    let fence = fencedRanges.at(fenceIndex)
    while (fence !== undefined && fence.end <= at) {
      fenceIndex += 1
      fence = fencedRanges.at(fenceIndex)
    }
    if (fence !== undefined && rangeContains(fence, at)) continue
    interruptions.push(at)
  }
  return interruptions
}

export function analyzeHtmlSource(
  node: MarkdownNode,
  markdown: MarkdownSource,
  parse: MarkdownParser,
  lexemePattern: RegExp,
): HtmlSourceAnalysis {
  const group = createHtmlSourceGroup(node, markdown)
  const value = node.value ?? ''
  const lexemes = [
    ...value.matchAll(new RegExp(lexemePattern.source, 'gi')),
  ].map((match) => ({ at: match.index, value: match[0] }))
  const maskedSource = maskHtmlLexemes(value, lexemes)
  const markdownTree = parse(maskedSource)
  const ranges = collectMarkdownCodeRanges(markdownTree.children)
  const fencedRanges = ranges.filter((range) => range.type === 'code')
  const interruptions = getCommonMarkHtmlInterruptions(
    value,
    lexemes,
    fencedRanges,
  )
  let interruptionIndex = 0
  const codeRanges = ranges.filter((range) => {
    if (range.type === 'code') return true
    let interruption = interruptions.at(interruptionIndex)
    while (interruption !== undefined && interruption < range.start) {
      interruptionIndex += 1
      interruption = interruptions.at(interruptionIndex)
    }
    return interruption === undefined || interruption >= range.end
  })

  return { codeRanges, group, lexemes, maskedSource }
}

export function sourceOffsetsForRange(
  group: HtmlSourceGroup,
  start: number,
  end: number,
): SourceOffsetMap {
  return group.offsets.slice(start, end + 1)
}

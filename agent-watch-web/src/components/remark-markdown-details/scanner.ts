const codeFencePattern = /^ {0,3}(`{3,}|~{3,})/
const openingDetailsPattern = /^<details(?=\s|>)/i
const closingDetailsPattern = /^<\/details\s*>/i
const rawTextOpeningPattern = /^<(script|style)(?=\s|>)/i
const otherHtmlTagPattern = /^<\/?[a-z][a-z\d:-]*(?:\s|\/?>)/i

type InlineCodeRange = { start: number; end: number }
type DetailsTag = { type: 'open' | 'close'; start: number; end: number }
type RawTextTag = 'script' | 'style'

function isEscaped(line: string, index: number) {
  let backslashes = 0
  for (
    let cursor = index - 1;
    cursor >= 0 && line[cursor] === '\\';
    cursor -= 1
  ) {
    backslashes += 1
  }
  return backslashes % 2 === 1
}

function backtickRunEnd(line: string, start: number) {
  let end = start
  while (line[end] === '`') end += 1
  return end
}

function stripBlockquotePrefixes(
  line: string,
  maxDepth = Number.POSITIVE_INFINITY,
) {
  let content = line
  let depth = 0
  while (depth < maxDepth) {
    const prefix = /^ {0,3}>[\t ]?/.exec(content)
    if (prefix === null) return { content, depth }
    content = content.slice(prefix[0].length)
    depth += 1
  }
  return { content, depth }
}

function stripListItemPrefix(line: string) {
  let content = line
  let indent = 0
  while (content.length > 0) {
    const prefix = /^ {0,3}(?:[-+*]|\d{1,9}[.)])[\t ]+/.exec(content)
    if (prefix === null) break
    content = content.slice(prefix[0].length)
    indent += prefix[0].length
  }
  return { content, indent: indent === 0 ? null : indent }
}

function stripIndentation(line: string, indent: number) {
  if (line.trim() === '') return ''
  const prefix = /^ */.exec(line)?.[0] ?? ''
  return prefix.length >= indent ? line.slice(indent) : null
}

function updateInlineMarkupState(
  line: string,
  state: {
    inHtmlComment: boolean
    rawTextTag: RawTextTag | null
    inlineCodeLength: number | null
  },
) {
  let { inHtmlComment, rawTextTag, inlineCodeLength } = state
  let cursor = 0
  while (cursor < line.length) {
    if (rawTextTag !== null) {
      const rawTextClosePattern = new RegExp(`</${rawTextTag}\\s*>`, 'ig')
      rawTextClosePattern.lastIndex = cursor
      const closing = rawTextClosePattern.exec(line)
      if (closing === null) break
      cursor = closing.index + closing[0].length
      rawTextTag = null
      continue
    }
    if (inHtmlComment) {
      const commentEnd = line.indexOf('-->', cursor)
      if (commentEnd < 0) break
      cursor = commentEnd + 3
      inHtmlComment = false
      continue
    }
    if (inlineCodeLength !== null) {
      const tick = line.indexOf('`', cursor)
      if (tick < 0) break
      if (isEscaped(line, tick)) {
        cursor = tick + 1
        continue
      }
      const runEnd = backtickRunEnd(line, tick)
      if (runEnd - tick === inlineCodeLength) inlineCodeLength = null
      cursor = runEnd
      continue
    }

    if (line[cursor] === '<') {
      if (line.startsWith('<!--', cursor)) {
        inHtmlComment = true
        cursor += 4
        continue
      }
      const suffix = line.slice(cursor)
      const rawTextOpening = rawTextOpeningPattern.exec(suffix)
      if (rawTextOpening !== null) {
        const end = tagEnd(line, cursor)
        if (end === null) break
        if (!/\/\s*>$/.test(line.slice(cursor, end))) {
          rawTextTag =
            rawTextOpening[1]?.toLowerCase() === 'script' ? 'script' : 'style'
        }
        cursor = end
        continue
      }
      if (otherHtmlTagPattern.test(suffix)) {
        const end = tagEnd(line, cursor)
        if (end === null) break
        cursor = end
        continue
      }
    }
    if (line[cursor] === '`' && !isEscaped(line, cursor)) {
      const runEnd = backtickRunEnd(line, cursor)
      inlineCodeLength = runEnd - cursor
      cursor = runEnd
      continue
    }
    cursor += 1
  }
  return { inHtmlComment, rawTextTag, inlineCodeLength }
}

function findCodeBlockLines(lines: readonly string[]) {
  const codeBlockLines: boolean[] = []
  let inHtmlComment = false
  let rawTextTag: RawTextTag | null = null
  let inlineCodeLength: number | null = null
  let fence: {
    marker: '`' | '~'
    length: number
    blockquoteDepth: number
    listIndent: number | null
  } | null = null
  for (const line of lines) {
    const { content: fenceContent, depth: blockquoteDepth } =
      stripBlockquotePrefixes(line)
    if (fence !== null) {
      const fencedBlockquote = stripBlockquotePrefixes(
        line,
        fence.blockquoteDepth,
      )
      const listContent =
        fencedBlockquote.depth !== fence.blockquoteDepth
          ? null
          : fence.listIndent === null
            ? fencedBlockquote.content
            : stripIndentation(fencedBlockquote.content, fence.listIndent)
      const closingFence = new RegExp(
        `^ {0,3}${fence.marker}{${String(fence.length)},}[\\t ]*\\r?$`,
      )
      if (listContent !== null) {
        codeBlockLines.push(true)
        if (closingFence.test(listContent)) fence = null
        continue
      }
      fence = null
    }

    if (inHtmlComment || rawTextTag !== null) {
      const state = updateInlineMarkupState(line, {
        inHtmlComment,
        rawTextTag,
        inlineCodeLength,
      })
      inHtmlComment = state.inHtmlComment
      rawTextTag = state.rawTextTag
      inlineCodeLength = state.inlineCodeLength
      codeBlockLines.push(false)
      continue
    }

    const { content: listContent, indent: listIndent } =
      stripListItemPrefix(fenceContent)
    const fenceMatch = codeFencePattern.exec(listContent)
    const fenceText = fenceMatch?.[1]
    if (fenceText !== undefined) {
      const marker = fenceText.startsWith('`') ? '`' : '~'
      fence = {
        marker,
        length: fenceText.length,
        blockquoteDepth,
        listIndent,
      }
      codeBlockLines.push(true)
      inlineCodeLength = null
      continue
    }
    const isIndentedCode = /^(?: {4,}|\t)/.test(listContent)
    codeBlockLines.push(isIndentedCode)
    if (isIndentedCode || listContent.trim() === '') {
      inlineCodeLength = null
      continue
    }
    const state = updateInlineMarkupState(listContent, {
      inHtmlComment,
      rawTextTag,
      inlineCodeLength,
    })
    inHtmlComment = state.inHtmlComment
    rawTextTag = state.rawTextTag
    inlineCodeLength = state.inlineCodeLength
  }
  return codeBlockLines
}

function appendInlineCodeRange(
  rangesByLine: InlineCodeRange[][],
  lines: readonly string[],
  opening: { line: number; start: number; length: number },
  closing: { line: number; end: number },
) {
  if (opening.line === closing.line) {
    rangesByLine[opening.line]?.push({ start: opening.start, end: closing.end })
    return
  }

  rangesByLine[opening.line]?.push({
    start: opening.start,
    end: lines[opening.line]?.length ?? opening.start,
  })
  for (let line = opening.line + 1; line < closing.line; line += 1) {
    rangesByLine[line]?.push({ start: 0, end: lines[line]?.length ?? 0 })
  }
  rangesByLine[closing.line]?.push({ start: 0, end: closing.end })
}

function findInlineCodeRangesByLine(
  lines: readonly string[],
  codeBlockLines: readonly boolean[],
) {
  const rangesByLine = lines.map(() => [] as InlineCodeRange[])
  let opening: { line: number; start: number; length: number } | null = null
  let inHtmlComment = false
  let rawTextTag: RawTextTag | null = null
  for (let lineIndex = 0; lineIndex < lines.length; lineIndex += 1) {
    const line = lines[lineIndex] ?? ''
    const isCodeBlockLine = codeBlockLines[lineIndex] === true
    if (isCodeBlockLine || line.trim() === '') {
      opening = null
      if (isCodeBlockLine || (rawTextTag === null && !inHtmlComment)) {
        continue
      }
    }

    let cursor = 0
    while (cursor < line.length) {
      if (rawTextTag !== null) {
        const rawTextClosePattern = new RegExp(`</${rawTextTag}\\s*>`, 'ig')
        rawTextClosePattern.lastIndex = cursor
        const closing = rawTextClosePattern.exec(line)
        if (closing === null) break
        cursor = closing.index + closing[0].length
        rawTextTag = null
        continue
      }
      if (inHtmlComment) {
        const commentEnd = line.indexOf('-->', cursor)
        if (commentEnd < 0) break
        cursor = commentEnd + 3
        inHtmlComment = false
        continue
      }
      if (opening !== null) {
        const tick = line.indexOf('`', cursor)
        if (tick < 0) break
        if (isEscaped(line, tick)) {
          cursor = tick + 1
          continue
        }
        const runEnd = backtickRunEnd(line, tick)
        if (runEnd - tick === opening.length) {
          appendInlineCodeRange(rangesByLine, lines, opening, {
            line: lineIndex,
            end: runEnd,
          })
          opening = null
        }
        cursor = runEnd
        continue
      }

      if (line[cursor] === '<') {
        if (line.startsWith('<!--', cursor)) {
          inHtmlComment = true
          cursor += 4
          continue
        }
        const suffix = line.slice(cursor)
        const rawTextOpening = rawTextOpeningPattern.exec(suffix)
        if (rawTextOpening !== null) {
          const end = tagEnd(line, cursor)
          if (end === null) break
          if (!/\/\s*>$/.test(line.slice(cursor, end))) {
            rawTextTag =
              rawTextOpening[1]?.toLowerCase() === 'script' ? 'script' : 'style'
          }
          cursor = end
          continue
        }
        if (otherHtmlTagPattern.test(suffix)) {
          const end = tagEnd(line, cursor)
          if (end === null) break
          cursor = end
          continue
        }
      }
      if (line[cursor] !== '`' || isEscaped(line, cursor)) {
        cursor += 1
        continue
      }

      const runEnd = backtickRunEnd(line, cursor)
      const runLength = runEnd - cursor
      opening = { line: lineIndex, start: cursor, length: runLength }
      cursor = runEnd
    }
  }
  return rangesByLine
}

function tagEnd(line: string, start: number) {
  let quote: '"' | "'" | null = null
  for (let index = start; index < line.length; index += 1) {
    const character = line[index]
    if (quote !== null) {
      if (character === quote) quote = null
    } else if (character === '"' || character === "'") {
      quote = character
    } else if (character === '>') {
      return index + 1
    }
  }
  return null
}

function findDetailsTags(
  line: string,
  startsInComment: boolean,
  codeRanges: readonly InlineCodeRange[],
  startsInRawText: RawTextTag | null,
) {
  const tags: DetailsTag[] = []
  let rangeIndex = 0
  let cursor = 0
  let inComment = startsInComment
  let rawTextTag = startsInRawText
  while (cursor < line.length) {
    if (rawTextTag !== null) {
      const rawTextClosePattern = new RegExp(`</${rawTextTag}\\s*>`, 'ig')
      rawTextClosePattern.lastIndex = cursor
      const closing = rawTextClosePattern.exec(line)
      if (closing === null) return { tags, inComment, rawTextTag }
      cursor = closing.index + closing[0].length
      rawTextTag = null
      continue
    }

    const codeRange = codeRanges[rangeIndex]
    if (codeRange !== undefined && cursor >= codeRange.end) {
      rangeIndex += 1
      continue
    }
    if (codeRange !== undefined && cursor >= codeRange.start) {
      cursor = codeRange.end
      rangeIndex += 1
      continue
    }
    if (inComment) {
      const commentEnd = line.indexOf('-->', cursor)
      if (commentEnd < 0) return { tags, inComment, rawTextTag }
      cursor = commentEnd + 3
      inComment = false
      continue
    }
    if (line[cursor] !== '<') {
      cursor += 1
      continue
    }

    const suffix = line.slice(cursor)
    if (suffix.startsWith('<!--')) {
      cursor += 4
      inComment = true
      continue
    }
    const rawTextOpening = rawTextOpeningPattern.exec(suffix)
    if (rawTextOpening !== null) {
      const end = tagEnd(line, cursor)
      if (end === null) break
      if (!/\/\s*>$/.test(line.slice(cursor, end))) {
        rawTextTag =
          rawTextOpening[1]?.toLowerCase() === 'script' ? 'script' : 'style'
      }
      cursor = end
      continue
    }
    const opening = openingDetailsPattern.exec(suffix)
    if (opening !== null) {
      const end = tagEnd(line, cursor)
      if (end === null) break
      tags.push({ type: 'open', start: cursor, end })
      cursor = end
      continue
    }

    const closing = closingDetailsPattern.exec(suffix)
    if (closing !== null) {
      tags.push({
        type: 'close',
        start: cursor,
        end: cursor + closing[0].length,
      })
      cursor += closing[0].length
      continue
    }
    if (otherHtmlTagPattern.test(suffix)) {
      const end = tagEnd(line, cursor)
      if (end === null) break
      cursor = end
      continue
    }
    cursor += 1
  }
  return { tags, inComment, rawTextTag }
}

function lineAt(lines: readonly string[], index: number) {
  return lines[index] ?? ''
}

export function findDetailsClose(lines: readonly string[], start: number) {
  let depth = 0
  let rawTextTag: RawTextTag | null = null
  let inHtmlComment = false
  const codeBlockLines = findCodeBlockLines(lines)
  const inlineCodeRanges = findInlineCodeRangesByLine(lines, codeBlockLines)

  for (let index = start; index < lines.length; index += 1) {
    const rawLine = lineAt(lines, index)
    if (rawTextTag === null && codeBlockLines[index] === true) continue

    const foundTags = findDetailsTags(
      rawLine,
      inHtmlComment,
      rawTextTag === null ? (inlineCodeRanges[index] ?? []) : [],
      rawTextTag,
    )
    inHtmlComment = foundTags.inComment
    rawTextTag = foundTags.rawTextTag
    for (const tag of foundTags.tags) {
      if (tag.type === 'open') {
        depth += 1
      } else {
        depth -= 1
        if (depth === 0) {
          return { line: index, start: tag.start, end: tag.end }
        }
      }
    }
  }

  return null
}

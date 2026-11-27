import type {
  MarkdownNode,
  MarkdownRoot,
} from '#components/markdown-message-types'

const detailsOpenPattern =
  /^<details(?:\s+open(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)?\s*>$/i
const detailsClosePattern = /^<\/details\s*>$/i
const summaryOpenPattern = /^<summary\s*>$/i
const summaryClosePattern = /^<\/summary\s*>$/i
const inlineSummaryPattern = /^<summary\s*>(.*?)<\/summary\s*>$/i
const codeFencePattern = /^ {0,3}(`{3,}|~{3,})/

type MarkdownProcessor = {
  parse: (value: string) => unknown
}
type MarkdownParser = (value: string) => MarkdownRoot

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isPositionOffset(value: unknown): value is { offset?: number } {
  return (
    isRecord(value) &&
    (value['offset'] === undefined || typeof value['offset'] === 'number')
  )
}

function isPosition(
  value: unknown,
): value is NonNullable<MarkdownNode['position']> {
  return (
    isRecord(value) &&
    isPositionOffset(value['start']) &&
    isPositionOffset(value['end'])
  )
}

function isMarkdownNode(value: unknown): value is MarkdownNode {
  if (!isRecord(value) || typeof value['type'] !== 'string') return false
  if (value['value'] !== undefined && typeof value['value'] !== 'string') {
    return false
  }
  if (value['url'] !== undefined && typeof value['url'] !== 'string') {
    return false
  }
  if (value['tagName'] !== undefined && typeof value['tagName'] !== 'string') {
    return false
  }
  if (value['position'] !== undefined && !isPosition(value['position'])) {
    return false
  }
  const children = value['children']
  return (
    children === undefined ||
    (Array.isArray(children) &&
      children.every((child: unknown) => isMarkdownNode(child)))
  )
}

function isMarkdownRoot(value: unknown): value is MarkdownRoot {
  return (
    isMarkdownNode(value) &&
    value.type === 'root' &&
    Array.isArray(value.children)
  )
}

function lineAt(lines: readonly string[], index: number) {
  return lines[index] ?? ''
}

function lineOffset(lines: readonly string[], index: number) {
  return lines
    .slice(0, index)
    .reduce((offset, line) => offset + line.length + 1, 0)
}

function shiftNodeOffsets(nodes: MarkdownNode[], offset: number) {
  for (const node of nodes) {
    if (node.position !== undefined) {
      const start = node.position.start.offset
      const end = node.position.end.offset
      node.position = {
        start: {
          ...node.position.start,
          ...(start === undefined ? {} : { offset: start + offset }),
        },
        end: {
          ...node.position.end,
          ...(end === undefined ? {} : { offset: end + offset }),
        },
      }
    }
    if (node.children !== undefined) shiftNodeOffsets(node.children, offset)
  }
}

function isOpeningTag(line: string, tagName: string) {
  return new RegExp(`^<${tagName}(?:\\s[^>]*)?>$`, 'i').test(line.trim())
}

function findDetailsClose(lines: readonly string[], start: number) {
  let depth = 0
  let fence: { marker: '`' | '~'; length: number } | null = null
  let rawTag: 'script' | 'style' | null = null

  for (let index = start; index < lines.length; index += 1) {
    const rawLine = lineAt(lines, index)
    const line = rawLine.trim()
    if (fence !== null) {
      const closingFence = new RegExp(
        `^${fence.marker}{${String(fence.length)},}\\s*$`,
      )
      if (closingFence.test(line)) fence = null
      continue
    }

    const fenceMatch = codeFencePattern.exec(rawLine)
    const fenceText = fenceMatch?.[1]
    if (fenceText !== undefined) {
      const marker = fenceText.startsWith('`') ? '`' : '~'
      fence = { marker, length: fenceText.length }
      continue
    }

    if (rawTag !== null) {
      if (new RegExp(`</${rawTag}\\s*>`, 'i').test(line)) rawTag = null
      continue
    }

    if (isOpeningTag(line, 'script')) {
      rawTag = 'script'
      continue
    }
    if (isOpeningTag(line, 'style')) {
      rawTag = 'style'
      continue
    }

    if (detailsOpenPattern.test(line)) depth += 1
    if (detailsClosePattern.test(line)) {
      depth -= 1
      if (depth === 0) return index
    }
  }

  return null
}

function parseSummary(source: string, parse: MarkdownParser) {
  const tree = parse(source)
  const paragraph = tree.children.find((node) => node.type === 'paragraph')
  return {
    type: 'summary',
    children: paragraph?.children ?? [{ type: 'text', value: source }],
    data: {
      hName: 'span',
      hProperties: { className: 'markdown-details-summary-content' },
    },
  } satisfies MarkdownNode
}

function parseDetailsSource(source: string, parse: MarkdownParser) {
  const lines = source.split('\n')
  const firstLine = lines.findIndex((line) => line.trim() !== '')
  if (firstLine < 0) return null

  const openTag = lineAt(lines, firstLine).trim()
  if (!detailsOpenPattern.test(openTag)) return null

  const closeLine = findDetailsClose(lines, firstLine)
  if (closeLine === null) return null

  let summaryStart = firstLine + 1
  while (
    summaryStart < closeLine &&
    lineAt(lines, summaryStart).trim() === ''
  ) {
    summaryStart += 1
  }

  let summaryEnd = summaryStart
  let summaryText = ''
  let summaryOffset = lineOffset(lines, summaryStart)
  const inlineSummary = inlineSummaryPattern.exec(
    lineAt(lines, summaryStart).trim(),
  )
  if (inlineSummary !== null) {
    summaryText = inlineSummary[1] ?? ''
    const column = lineAt(lines, summaryStart).indexOf(summaryText)
    summaryOffset += Math.max(column, 0)
  } else if (summaryOpenPattern.test(lineAt(lines, summaryStart).trim())) {
    const closeSummary = lines.findIndex(
      (line, index) =>
        index > summaryStart && summaryClosePattern.test(line.trim()),
    )
    if (closeSummary < 0 || closeSummary >= closeLine) return null
    summaryEnd = closeSummary
    summaryText = lines
      .slice(summaryStart + 1, closeSummary)
      .join('\n')
      .trim()
    const contentStart = lineOffset(lines, summaryStart + 1)
    summaryOffset = summaryText.length
      ? source.indexOf(summaryText, contentStart)
      : contentStart
    if (summaryOffset < 0) summaryOffset = contentStart
  } else {
    return null
  }

  const bodyStart = lineOffset(lines, summaryEnd + 1)
  const rawBody = source.slice(bodyStart, lineOffset(lines, closeLine))
  const bodyLeadingWhitespace = rawBody.length - rawBody.trimStart().length
  const bodySource = rawBody.trim()
  const bodyTree = parse(bodySource)
  const body = transformNodes(bodyTree.children, parse, bodySource)
  shiftNodeOffsets(body, bodyLeadingWhitespace + bodyStart)
  const summary = parseSummary(summaryText, parse)
  shiftNodeOffsets(summary.children, summaryOffset)
  const openAttribute = /^<details\s+open(?:\s*=|\s*>)/i.test(openTag)

  return {
    type: 'details',
    children: [summary, ...body],
    data: {
      hName: 'details',
      ...(openAttribute ? { hProperties: { open: true } } : {}),
    },
  } satisfies MarkdownNode
}

function transformNodes(
  nodes: MarkdownNode[],
  parse: MarkdownParser,
  source: string,
) {
  const result: MarkdownNode[] = []
  for (let index = 0; index < nodes.length; index += 1) {
    const node = nodes[index]
    if (node === undefined) continue
    if (node.type === 'html' && node.value !== undefined) {
      const openTag = (node.value.split('\n', 1)[0] ?? '').trim()
      const start = node.position?.start.offset
      if (start === undefined || !detailsOpenPattern.test(openTag)) {
        result.push(node)
        continue
      }

      const suffixLines = source.slice(start).split('\n')
      const closeLine = findDetailsClose(suffixLines, 0)
      const detailsSource =
        closeLine === null
          ? null
          : suffixLines.slice(0, closeLine + 1).join('\n')
      const details =
        detailsSource === null ? null : parseDetailsSource(detailsSource, parse)
      if (details !== null) {
        shiftNodeOffsets([details], start)
        result.push(details)
        const end = start + (detailsSource?.length ?? 0)
        while (
          index + 1 < nodes.length &&
          (nodes[index + 1]?.position?.start.offset ?? end) < end
        ) {
          index += 1
        }
        continue
      }
    }

    if (node.children !== undefined) {
      node.children = transformNodes(node.children, parse, source)
    }
    result.push(node)
  }
  return result
}

export function remarkMarkdownDetails(this: MarkdownProcessor) {
  const parse: MarkdownParser = (value) => {
    const tree = this.parse(value)
    return isMarkdownRoot(tree) ? tree : { type: 'root', children: [] }
  }
  return (tree: MarkdownRoot, file: { value: string | Uint8Array }) => {
    const source =
      typeof file.value === 'string'
        ? file.value
        : new TextDecoder().decode(file.value)
    tree.children = transformNodes(tree.children, parse, source)
  }
}

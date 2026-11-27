import type {
  MarkdownNode,
  MarkdownRoot,
} from '#components/markdown-message-types'
import { findDetailsClose } from '#components/remark-markdown-details/scanner'
import type {
  MarkdownParser,
  ReferenceDefinitionSource,
} from '#components/remark-markdown-details/source'
import {
  getReferenceDefinitionSources,
  parseMarkdownSection,
  shiftMarkdownNodeOffsets,
  trimBlankLines,
} from '#components/remark-markdown-details/source'

const detailsOpenTagPattern =
  /^<details(?:\s+open(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)?\s*>/i
const summaryOpenPattern = /^<summary\s*>$/i
const summaryClosePattern = /^<\/summary\s*>$/i
const inlineSummaryPrefixPattern = /^<summary\s*>(.*?)<\/summary\s*>/i
// The body parser reruns for each wrapper, so bound its work on nested input.
const maxDetailsNesting = 32

type MarkdownProcessor = {
  parse: (value: string) => unknown
}

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

function parseSummary(
  source: string,
  parse: MarkdownParser,
  referenceDefinitions: readonly ReferenceDefinitionSource[],
  sourceOffset: number,
) {
  const { tree } = parseMarkdownSection(
    source,
    parse,
    referenceDefinitions,
    sourceOffset,
  )
  const paragraphs = tree.children.filter((node) => node.type === 'paragraph')
  const children = paragraphs.flatMap((paragraph, index) =>
    index === 0
      ? (paragraph.children ?? [])
      : [{ type: 'break' }, ...(paragraph.children ?? [])],
  )
  return {
    type: 'summary',
    children:
      children.length === 0 ? [{ type: 'text', value: source }] : children,
    data: {
      hName: 'span',
      hProperties: { className: 'markdown-details-summary-content' },
    },
  } satisfies MarkdownNode
}

function parseInlineSummary(value: string) {
  const match = inlineSummaryPrefixPattern.exec(value)
  if (match === null) return null

  const summaryTag = /^<summary\s*>/i.exec(match[0])
  if (summaryTag === null) return null

  return {
    content: match[1] ?? '',
    contentOffset: summaryTag[0].length,
    consumedLength: match[0].length,
  }
}

function parseDetailsSource(
  source: string,
  parse: MarkdownParser,
  nestingLevel: number,
  referenceDefinitions: readonly ReferenceDefinitionSource[],
  sourceOffset: number,
) {
  const lines = source.split('\n')
  const firstLine = lines.findIndex((line) => line.trim() !== '')
  if (firstLine < 0) return null

  const openLine = lineAt(lines, firstLine)
  const openLineContent = openLine.trimStart()
  const openTagMatch = detailsOpenTagPattern.exec(openLineContent)
  if (openTagMatch === null) return null
  const openTag = openTagMatch[0]
  const openingRemainder = openLineContent.slice(openTag.length)
  const openingRemainderContent = openingRemainder.trimStart()
  const openingRemainderWhitespace =
    openingRemainder.length - openingRemainderContent.length
  const inlineOpeningSummary = parseInlineSummary(openingRemainderContent)

  const close = findDetailsClose(lines, firstLine)
  if (close === null) return null
  const closeLine = close.line

  let summaryStart = firstLine + 1
  while (
    summaryStart < closeLine &&
    lineAt(lines, summaryStart).trim() === ''
  ) {
    summaryStart += 1
  }

  let summaryEnd = summaryStart
  let summaryText = ''
  let summaryOffset: number
  let bodyStart: number
  if (inlineOpeningSummary !== null) {
    const openingLineOffset =
      lineOffset(lines, firstLine) + (openLine.length - openLineContent.length)
    summaryText = inlineOpeningSummary.content
    summaryOffset =
      openingLineOffset +
      openTag.length +
      openingRemainderWhitespace +
      inlineOpeningSummary.contentOffset
    summaryEnd = firstLine
    bodyStart =
      openingLineOffset +
      openTag.length +
      openingRemainderWhitespace +
      inlineOpeningSummary.consumedLength
  } else {
    if (openingRemainderContent !== '') return null
    const summaryLine = lineAt(lines, summaryStart)
    const summaryLineContent = summaryLine.trim()
    const inlineSummary = parseInlineSummary(summaryLineContent)
    if (
      inlineSummary !== null &&
      inlineSummary.consumedLength === summaryLineContent.length
    ) {
      summaryText = inlineSummary.content
      summaryOffset =
        lineOffset(lines, summaryStart) +
        (summaryLine.length - summaryLine.trimStart().length) +
        inlineSummary.contentOffset
    } else if (summaryOpenPattern.test(summaryLine.trim())) {
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
    bodyStart = lineOffset(lines, summaryEnd + 1)
  }

  const bodyOffset = lineOffset(lines, closeLine) + close.start
  const rawBody = source.slice(bodyStart, bodyOffset)
  const trimmedBody = trimBlankLines(rawBody)
  const bodySource = trimmedBody.source
  const bodySourceOffset = sourceOffset + bodyStart + trimmedBody.offset
  const parsedBody = parseMarkdownSection(
    bodySource,
    parse,
    referenceDefinitions,
    bodySourceOffset,
  )
  const bodyReferenceDefinitions = [
    ...referenceDefinitions,
    ...getReferenceDefinitionSources(parsedBody.tree.children, bodySource).map(
      (definition) => ({
        ...definition,
        offset: definition.offset + bodySourceOffset,
      }),
    ),
  ].sort((left, right) => left.offset - right.offset)
  const body = transformNodes(
    parsedBody.tree.children,
    parse,
    parsedBody.source,
    nestingLevel + 1,
    bodyReferenceDefinitions,
    bodySourceOffset,
  )
  shiftMarkdownNodeOffsets(body, trimmedBody.offset + bodyStart)
  const summary = parseSummary(
    summaryText,
    parse,
    bodyReferenceDefinitions,
    sourceOffset + summaryOffset,
  )
  shiftMarkdownNodeOffsets(summary.children, summaryOffset)
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
  nestingLevel = 0,
  referenceDefinitions: readonly ReferenceDefinitionSource[] = [],
  sourceOffset = 0,
) {
  const result: MarkdownNode[] = []
  for (let index = 0; index < nodes.length; index += 1) {
    const node = nodes[index]
    if (node === undefined) continue
    if (
      nestingLevel < maxDetailsNesting &&
      node.type === 'html' &&
      node.value !== undefined
    ) {
      const openTag = (node.value.split('\n', 1)[0] ?? '').trim()
      const start = node.position?.start.offset
      if (start === undefined || !detailsOpenTagPattern.test(openTag)) {
        result.push(node)
        continue
      }

      const suffixLines = source.slice(start).split('\n')
      const close = findDetailsClose(suffixLines, 0)
      const detailsLength =
        close === null ? null : lineOffset(suffixLines, close.line) + close.end
      const detailsSource =
        detailsLength === null
          ? null
          : source.slice(start, start + detailsLength)
      const details =
        detailsSource === null
          ? null
          : parseDetailsSource(
              detailsSource,
              parse,
              nestingLevel,
              referenceDefinitions,
              sourceOffset + start,
            )
      if (details !== null) {
        shiftMarkdownNodeOffsets([details], start)
        result.push(details)
        const detailsEnd = start + (detailsSource?.length ?? 0)
        let nodeEnd = Math.max(
          detailsEnd,
          node.position?.end.offset ?? detailsEnd,
        )
        let consumedIndex = index + 1
        while (consumedIndex < nodes.length) {
          const consumedNode = nodes[consumedIndex]
          const consumedStart = consumedNode?.position?.start.offset
          if (consumedStart === undefined || consumedStart >= nodeEnd) break
          nodeEnd = Math.max(
            nodeEnd,
            consumedNode.position?.end.offset ?? nodeEnd,
          )
          consumedIndex += 1
        }
        const unconsumedSource = source.slice(detailsEnd, nodeEnd)
        if (unconsumedSource.length > 0) {
          const parsedSuffix = parseMarkdownSection(
            unconsumedSource,
            parse,
            referenceDefinitions,
            sourceOffset + detailsEnd,
          )
          const suffixNodes = transformNodes(
            parsedSuffix.tree.children,
            parse,
            parsedSuffix.source,
            nestingLevel,
            referenceDefinitions,
            sourceOffset + detailsEnd,
          )
          shiftMarkdownNodeOffsets(suffixNodes, detailsEnd)
          result.push(...suffixNodes)
        }
        index = consumedIndex - 1
        continue
      }
    }

    if (node.children !== undefined) {
      node.children = transformNodes(
        node.children,
        parse,
        source,
        nestingLevel,
        referenceDefinitions,
        sourceOffset,
      )
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
    const referenceDefinitions = getReferenceDefinitionSources(
      tree.children,
      source,
    )
    tree.children = transformNodes(
      tree.children,
      parse,
      source,
      0,
      referenceDefinitions,
      0,
    )
  }
}

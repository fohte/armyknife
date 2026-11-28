import type {
  MarkdownNode,
  MarkdownRoot,
} from '#components/markdown-message-types'
import {
  analyzeHtmlSource,
  createMarkdownSource,
  type HtmlSourceAnalysis,
  type MarkdownSource,
  sourceOffsetsForRange,
} from '#components/remark-markdown-details/html-source'
import {
  getSupportedDetailsIntervals,
  renderMarkdownDetails,
} from '#components/remark-markdown-details/render'
import {
  getReferenceDefinitionSources,
  type MarkdownParser,
  type ReferenceDefinitionSource,
  type SourceOffsetMap,
  visitMarkdownNodes,
} from '#components/remark-markdown-details/source'

const htmlCandidateStart = /^\s*<\/?(?:details|summary)\b/i
const htmlLexemePattern =
  /<!--[\s\S]*?(?:-->|$)|<(script|style|pre|textarea)\b[\s\S]*?(?:<\/\1\s*>|$)|<\/?(?:details|summary)\b(?:[^>"']|"[^"]*"|'[^']*')*>/gi

type Processor = { parse: (value: string) => unknown }
type HiddenDefinition = {
  end: number
  node: MarkdownNode
  reference: ReferenceDefinitionSource
}

function isRoot(value: unknown): value is MarkdownRoot {
  return (
    typeof value === 'object' &&
    value !== null &&
    'type' in value &&
    value.type === 'root' &&
    'children' in value &&
    Array.isArray(value.children)
  )
}

function getDefinitionNodes(nodes: readonly MarkdownNode[]) {
  const definitions: MarkdownNode[] = []
  visitMarkdownNodes(nodes, (node) => {
    if (node.type === 'definition') definitions.push(node)
  })
  return definitions
}

function mapDefinitionOffsets(
  node: MarkdownNode,
  offsets: SourceOffsetMap,
  positionOffset: number,
) {
  const pending = [node]
  while (pending.length > 0) {
    const current = pending.pop()
    if (current === undefined) continue
    if (current.position !== undefined) {
      const { start, end } = current.position
      current.position = {
        start: {
          ...start,
          ...(start.offset === undefined ||
          offsets[start.offset - positionOffset] === undefined
            ? {}
            : { offset: offsets[start.offset - positionOffset] }),
        },
        end: {
          ...end,
          ...(end.offset === undefined ||
          offsets[end.offset - positionOffset] === undefined
            ? {}
            : { offset: offsets[end.offset - positionOffset] }),
        },
      }
    }
    if (current.children !== undefined) {
      for (const child of current.children) pending.push(child)
    }
  }
}

function getHtmlDefinitionCandidates(
  analysis: HtmlSourceAnalysis,
  parse: MarkdownParser,
) {
  const definitions: HiddenDefinition[] = []
  const value = analysis.group.node.value ?? ''
  let cursor = 0
  let codeRangeIndex = 0
  let inSummary = false
  const appendDefinitions = (end: number) => {
    const section = value.slice(cursor, end)
    if (section.trim() === '' || inSummary) return
    const prefix = '\n\n'
    for (const definition of getDefinitionNodes(
      parse(`${prefix}${section}`).children,
    )) {
      const parsedStart = definition.position?.start.offset
      const parsedEnd = definition.position?.end.offset
      if (parsedStart === undefined || parsedEnd === undefined) continue
      const start = parsedStart - prefix.length
      const finish = parsedEnd - prefix.length
      if (start < 0 || finish > section.length) continue
      const localStart = cursor + start
      const localEnd = cursor + finish
      const offsets = sourceOffsetsForRange(
        analysis.group,
        localStart,
        localEnd,
      )
      mapDefinitionOffsets(definition, offsets, parsedStart)
      definitions.push({
        node: definition,
        end:
          offsets[offsets.length - 1] ??
          analysis.group.offsets[localEnd] ??
          localEnd,
        reference: {
          offset:
            offsets[0] ?? analysis.group.offsets[localStart] ?? localStart,
          source: section.slice(start, finish),
        },
      })
    }
  }

  for (const lexeme of analysis.lexemes) {
    while (
      analysis.codeRanges[codeRangeIndex] !== undefined &&
      analysis.codeRanges[codeRangeIndex].end <= lexeme.at
    ) {
      codeRangeIndex += 1
    }
    const codeRange = analysis.codeRanges[codeRangeIndex]
    if (
      codeRange !== undefined &&
      codeRange.start <= lexeme.at &&
      lexeme.at < codeRange.end
    ) {
      continue
    }

    appendDefinitions(lexeme.at)
    cursor = lexeme.at + lexeme.value.length
    if (/^<summary\s*>$/i.test(lexeme.value)) inSummary = true
    if (/^<\/summary\s*>$/i.test(lexeme.value)) inSummary = false
  }
  appendDefinitions(value.length)
  return definitions
}

function analyzeHtmlGroups(
  nodes: MarkdownNode[],
  parse: MarkdownParser,
  markdown: MarkdownSource,
  analyses: Map<MarkdownNode, HtmlSourceAnalysis>,
  hiddenDefinitions: HiddenDefinition[],
) {
  for (const node of nodes) {
    if (
      node.type === 'html' &&
      node.value !== undefined &&
      htmlCandidateStart.test(node.value)
    ) {
      analyses.set(
        node,
        analyzeHtmlSource(node, markdown, parse, htmlLexemePattern),
      )
    }
  }

  const intervals = getSupportedDetailsIntervals(nodes, parse, analyses)
  let intervalIndex = 0
  for (const node of nodes) {
    const analysis = analyses.get(node)
    if (analysis !== undefined) {
      for (const definition of getHtmlDefinitionCandidates(analysis, parse)) {
        const offset = definition.reference.offset
        while (
          intervals[intervalIndex] !== undefined &&
          intervals[intervalIndex].end < offset
        ) {
          intervalIndex += 1
        }
        const interval = intervals[intervalIndex]
        if (
          interval === undefined ||
          interval.start > offset ||
          definition.end > interval.end
        ) {
          continue
        }
        hiddenDefinitions.push(definition)
      }
    }
    if (node.children !== undefined) {
      analyzeHtmlGroups(
        node.children,
        parse,
        markdown,
        analyses,
        hiddenDefinitions,
      )
    }
  }
}

function removeDefinitionNodes(
  nodes: MarkdownNode[],
  definitions: MarkdownNode[],
): MarkdownNode[] {
  const remaining: MarkdownNode[] = []
  for (const node of nodes) {
    if (node.type === 'definition') {
      definitions.push(node)
      continue
    }
    if (node.children !== undefined) {
      node.children = removeDefinitionNodes(node.children, definitions)
    }
    remaining.push(node)
  }
  return remaining
}

function mergeByOffset<T>(
  left: readonly T[],
  right: readonly T[],
  offset: (value: T) => number,
) {
  const merged: T[] = []
  let leftIndex = 0
  let rightIndex = 0
  while (leftIndex < left.length || rightIndex < right.length) {
    const leftValue = left[leftIndex]
    const rightValue = right[rightIndex]
    if (
      rightValue === undefined ||
      (leftValue !== undefined && offset(leftValue) <= offset(rightValue))
    ) {
      if (leftValue !== undefined) merged.push(leftValue)
      leftIndex += 1
    } else {
      merged.push(rightValue)
      rightIndex += 1
    }
  }
  return merged
}

function orderDefinitions(definitions: MarkdownNode[]) {
  const unique: MarkdownNode[] = []
  const identifiers = new Set<string>()
  for (const definition of definitions) {
    const identifier = definition.identifier?.toUpperCase()
    if (identifier !== undefined && identifiers.has(identifier)) continue
    if (identifier !== undefined) identifiers.add(identifier)
    unique.push(definition)
  }
  return unique
}

export function remarkMarkdownDetails(this: Processor) {
  const parse: MarkdownParser = (value) => {
    const tree = this.parse(value)
    return isRoot(tree) ? tree : { type: 'root', children: [] }
  }
  return (tree: MarkdownRoot, file: { value: string | Uint8Array }) => {
    const source =
      typeof file.value === 'string'
        ? file.value
        : new TextDecoder().decode(file.value)
    const markdown = createMarkdownSource(source)
    const visibleReferences = getReferenceDefinitionSources(
      tree.children,
      source,
    )
    const analyses = new Map<MarkdownNode, HtmlSourceAnalysis>()
    const hiddenDefinitions: HiddenDefinition[] = []
    analyzeHtmlGroups(
      tree.children,
      parse,
      markdown,
      analyses,
      hiddenDefinitions,
    )
    const hiddenReferences = hiddenDefinitions.map((item) => item.reference)
    const references = mergeByOffset(
      visibleReferences,
      hiddenReferences,
      (reference) => reference.offset,
    )
    const hiddenNodes = hiddenDefinitions.map((item) => item.node)
    const visibleNodes: MarkdownNode[] = []
    tree.children = removeDefinitionNodes(tree.children, visibleNodes)
    const definitions = mergeByOffset(
      visibleNodes,
      hiddenNodes,
      (definition) =>
        definition.position?.start.offset ?? Number.MAX_SAFE_INTEGER,
    )
    tree.children = [...orderDefinitions(definitions), ...tree.children]
    tree.children = renderMarkdownDetails(
      tree.children,
      parse,
      references,
      analyses,
      markdown,
      hiddenDefinitions.length > 0,
    )
  }
}

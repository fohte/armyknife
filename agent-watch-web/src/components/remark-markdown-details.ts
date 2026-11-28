import type {
  MarkdownNode,
  MarkdownRoot,
} from '#components/markdown-message-types'
import {
  analyzeHtmlSource,
  createMarkdownSource,
  type HtmlSourceAnalysis,
  type MarkdownSource,
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
} from '#components/remark-markdown-details/source'

const htmlCandidateStart = /^\s*<\/?(?:details|summary)\b/i
const htmlLexemePattern =
  /<!--[\s\S]*?(?:-->|$)|<(script|style|pre|textarea)\b[\s\S]*?(?:<\/\1\s*>|$)|<\/?(?:details|summary)\b(?:[^>"']|"[^"]*"|'[^']*')*>/gi

type Processor = { parse: (value: string) => unknown }
type HiddenDefinition = {
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
  const pending = [...nodes].reverse()
  while (pending.length > 0) {
    const node = pending.pop()
    if (node === undefined) continue
    if (node.type === 'definition') definitions.push(node)
    if (node.children !== undefined) {
      for (let index = node.children.length - 1; index >= 0; index -= 1) {
        const child = node.children[index]
        if (child !== undefined) pending.push(child)
      }
    }
  }
  return definitions
}

function mapDefinitionOffsets(node: MarkdownNode, offsets: SourceOffsetMap) {
  const pending = [node]
  while (pending.length > 0) {
    const current = pending.pop()
    if (current === undefined) continue
    if (current.position !== undefined) {
      const { start, end } = current.position
      current.position = {
        start: {
          ...start,
          ...(start.offset === undefined || offsets[start.offset] === undefined
            ? {}
            : { offset: offsets[start.offset] }),
        },
        end: {
          ...end,
          ...(end.offset === undefined || offsets[end.offset] === undefined
            ? {}
            : { offset: offsets[end.offset] }),
        },
      }
    }
    if (current.children !== undefined) {
      for (const child of current.children) pending.push(child)
    }
  }
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
      for (const definition of getDefinitionNodes(
        analysis.markdownTree.children,
      )) {
        const localStart = definition.position?.start.offset
        const localEnd = definition.position?.end.offset
        if (localStart === undefined || localEnd === undefined) continue
        const offset = analysis.group.offsets[localStart]
        const end = analysis.group.offsets[localEnd]
        while (
          intervals[intervalIndex] !== undefined &&
          intervals[intervalIndex].end < (offset ?? Number.MAX_SAFE_INTEGER)
        ) {
          intervalIndex += 1
        }
        const interval = intervals[intervalIndex]
        if (
          offset === undefined ||
          end === undefined ||
          interval === undefined ||
          interval.start > offset ||
          end > interval.end
        ) {
          continue
        }

        const source = analysis.maskedSource.slice(localStart, localEnd)
        mapDefinitionOffsets(definition, analysis.group.offsets)
        hiddenDefinitions.push({
          node: definition,
          reference: { offset, source },
        })
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
    const visibleReferences = getReferenceDefinitionSources(
      tree.children,
      source,
    )
    const analyses = new Map<MarkdownNode, HtmlSourceAnalysis>()
    const hiddenDefinitions: HiddenDefinition[] = []
    analyzeHtmlGroups(
      tree.children,
      parse,
      createMarkdownSource(source),
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
    )
  }
}

import type { MarkdownNode } from '#components/markdown-message-types'
import type {
  HtmlSourceAnalysis,
  MarkdownSource,
} from '#components/remark-markdown-details/html-source'
import {
  type MarkdownParser,
  parseMarkdownSection,
  type ReferenceDefinitionSource,
} from '#components/remark-markdown-details/source'

export function findReferenceCandidateNodes(nodes: readonly MarkdownNode[]) {
  const candidates = new Set<MarkdownNode>()
  const hasCandidate = new Map<MarkdownNode, boolean>()
  const pending = nodes.map((node) => ({ node, visited: false })).reverse()
  while (pending.length > 0) {
    const item = pending.pop()
    if (item === undefined) continue
    const { node, visited } = item
    if (
      node.type === 'code' ||
      node.type === 'inlineCode' ||
      node.type === 'html'
    ) {
      hasCandidate.set(node, false)
      continue
    }
    if (!visited) {
      pending.push({ node, visited: true })
      for (const child of [...(node.children ?? [])].reverse()) {
        pending.push({ node: child, visited: false })
      }
      continue
    }
    const ownCandidate =
      node.type === 'text' && /\[[^\]]+\]/.test(node.value ?? '')
    const hasChildCandidate =
      node.children?.some((child) => hasCandidate.get(child) === true) ?? false
    const result = ownCandidate || hasChildCandidate
    hasCandidate.set(node, result)
    if (result) candidates.add(node)
  }
  return candidates
}

export function createHtmlAnalysisOffsetMap(
  analyses: ReadonlyMap<MarkdownNode, HtmlSourceAnalysis>,
) {
  const byOffset = new Map<number, HtmlSourceAnalysis>()
  for (const analysis of analyses.values()) {
    const offset = analysis.group.node.position?.start.offset
    if (offset !== undefined) byOffset.set(offset, analysis)
    byOffset.set(analysis.group.offset, analysis)
  }
  return byOffset
}

export function reparseReferenceBlock(
  node: MarkdownNode,
  hasCandidate: boolean,
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
  markdown: MarkdownSource,
) {
  if (
    references.length === 0 ||
    !hasCandidate ||
    !(
      node.type === 'paragraph' ||
      node.type === 'heading' ||
      node.type === 'blockquote' ||
      node.type === 'list' ||
      node.type === 'table'
    )
  ) {
    return null
  }
  const start = node.position?.start.offset
  const end = node.position?.end.offset
  if (start === undefined || end === undefined) return null
  return parseMarkdownSection(
    markdown.source.slice(start, end),
    parse,
    references,
    start,
    undefined,
    true,
  )
}

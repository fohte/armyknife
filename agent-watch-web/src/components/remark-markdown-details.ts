import type {
  MarkdownNode,
  MarkdownRoot,
} from '#components/markdown-message-types'
import {
  getReferenceDefinitionSources,
  type MarkdownParser,
  parseMarkdownSection,
  type ReferenceDefinitionSource,
  shiftMarkdownNodeOffsets,
} from '#components/remark-markdown-details/source'

const detailsOpenPattern =
  /^<details(?:\s+open(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)?\s*>$/i
const detailsStartPattern =
  /^\s*<details(?:\s+open(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)?\s*>/i
const detailsClosePattern = /^<\/details\s*>$/i
const summaryOpenPattern = /^<summary\s*>$/i
const summaryClosePattern = /^<\/summary\s*>$/i
const htmlLexemePattern =
  /<!--[\s\S]*?(?:-->|$)|<(script|style|pre|textarea)\b[\s\S]*?(?:<\/\1\s*>|$)|<\/?(?:details|summary)\b(?:[^>"']|"[^"]*"|'[^']*')*>/gi

type MarkdownProcessor = { parse: (value: string) => unknown }

type Token = {
  kind:
    'text' | 'details-open' | 'details-close' | 'summary-open' | 'summary-close'
  value: string
  offset: number
  group: HtmlGroup
  pair?: Token
  summaryOpen?: Token
  summaryClose?: Token
  matched?: boolean
  index?: number
  pairIndex?: number
  summaryOpenIndex?: number
  summaryCloseIndex?: number
  afterMatchedClose?: boolean
}

type HtmlGroup = {
  node: MarkdownNode
  tokens: Token[]
}

type Unit = { node: MarkdownNode } | { token: Token }

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isRoot(value: unknown): value is MarkdownRoot {
  return (
    isRecord(value) &&
    value['type'] === 'root' &&
    Array.isArray(value['children'])
  )
}

function tokenizeHtml(node: MarkdownNode): HtmlGroup | null {
  if (node.type !== 'html' || node.value === undefined) return null
  const value = node.value
  const firstTag = /^\s*(<[^>]+>)/.exec(value)?.[1]
  if (
    firstTag === undefined ||
    (!detailsStartPattern.test(value) &&
      !detailsClosePattern.test(firstTag) &&
      !summaryOpenPattern.test(firstTag))
  ) {
    return null
  }

  const group: HtmlGroup = { node, tokens: [] }
  const add = (kind: Token['kind'], value: string, offset: number) => {
    group.tokens.push({ kind, value, offset, group })
  }
  let cursor = 0
  htmlLexemePattern.lastIndex = 0
  for (const match of value.matchAll(htmlLexemePattern)) {
    const tag = match[0]
    if (
      tag.startsWith('<!--') ||
      /^<(?:script|style|pre|textarea)\b/i.test(tag)
    ) {
      continue
    }
    const matchIndex = match.index
    const openDetails = detailsOpenPattern.test(tag)
    const closeDetails = detailsClosePattern.test(tag)
    const openSummary = summaryOpenPattern.test(tag)
    const closeSummary = summaryClosePattern.test(tag)
    const kind = openDetails
      ? 'details-open'
      : closeDetails
        ? 'details-close'
        : openSummary
          ? 'summary-open'
          : closeSummary
            ? 'summary-close'
            : null
    if (kind === null) {
      continue
    }
    if (matchIndex > cursor)
      add('text', value.slice(cursor, matchIndex), cursor)
    add(kind, tag, matchIndex)
    cursor = matchIndex + tag.length
  }
  if (cursor < value.length) add('text', value.slice(cursor), cursor)
  return group
}

function toUnits(
  nodes: MarkdownNode[],
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
): Unit[] {
  const units: Unit[] = []
  for (const node of nodes) {
    if (node.children !== undefined) {
      node.children = transformNodes(node.children, parse, references)
    }
    const group = tokenizeHtml(node)
    if (group === null) units.push({ node })
    else for (const token of group.tokens) units.push({ token })
  }
  return units
}

function pairDetails(units: Unit[]) {
  const stack: Array<{
    open: Token
    summaryOpen?: Token
    summaryClose?: Token
  }> = []
  for (const [index, unit] of units.entries()) {
    if (!('token' in unit)) continue
    const token = unit.token
    token.index = index
    if (token.kind === 'details-open') {
      stack.push({ open: token })
    } else if (token.kind === 'summary-open') {
      const frame = stack.at(-1)
      if (frame !== undefined && frame.summaryOpen === undefined) {
        frame.summaryOpen = token
      }
    } else if (token.kind === 'summary-close') {
      const frame = stack.at(-1)
      if (
        frame?.summaryOpen !== undefined &&
        frame.summaryClose === undefined
      ) {
        frame.summaryClose = token
      }
    } else if (token.kind === 'details-close') {
      const frame = stack.pop()
      if (
        frame?.summaryOpen !== undefined &&
        frame.summaryClose !== undefined &&
        frame.summaryOpen.index !== undefined &&
        frame.summaryClose.index !== undefined &&
        frame.open.index !== undefined
      ) {
        frame.open.pair = token
        frame.open.pairIndex = index
        frame.open.summaryOpen = frame.summaryOpen
        frame.open.summaryClose = frame.summaryClose
        frame.open.summaryOpenIndex = frame.summaryOpen.index
        frame.open.summaryCloseIndex = frame.summaryClose.index
        token.pair = frame.open
        token.pairIndex = frame.open.index
        token.matched = true
        frame.open.matched = true
      }
    }
  }
  const afterClose = new Map<HtmlGroup, boolean>()
  for (const unit of units) {
    if (!('token' in unit)) continue
    const token = unit.token
    if (token.kind === 'details-close' && token.matched === true) {
      afterClose.set(token.group, true)
    }
    token.afterMatchedClose = afterClose.get(token.group) ?? false
  }
}

function markdownNodes(
  source: string,
  offset: number,
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
) {
  if (source.trim() === '') return []
  const parsed = parseMarkdownSection(source, parse, references, offset)
  shiftMarkdownNodeOffsets(parsed.tree.children, offset)
  return transformNodes(parsed.tree.children, parse, references)
}

function parseSummary(
  units: Unit[],
  start: number,
  end: number,
  source: string,
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
) {
  const children: MarkdownNode[] = []
  const appendParagraphs = (nodes: MarkdownNode[]) => {
    for (const node of nodes) {
      if (node.type !== 'paragraph') continue
      if (children.length > 0) children.push({ type: 'break' })
      children.push(...(node.children ?? []))
    }
  }
  for (let index = start; index < end; index += 1) {
    const unit = units[index]
    if (unit === undefined) continue
    if ('token' in unit && unit.token.kind === 'text') {
      appendParagraphs(
        markdownNodes(
          unit.token.value,
          (unit.token.group.node.position?.start.offset ?? 0) +
            unit.token.offset,
          parse,
          references,
        ),
      )
    } else if (!('token' in unit) && unit.node.type === 'paragraph') {
      appendParagraphs([unit.node])
    }
  }
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

function renderUnits(
  units: Unit[],
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
  markdownContext: boolean,
): MarkdownNode[] {
  pairDetails(units)
  return renderRange(units, 0, units.length, parse, references, markdownContext)
}

function renderRange(
  units: Unit[],
  start: number,
  end: number,
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
  markdownContext: boolean,
): MarkdownNode[] {
  const result: MarkdownNode[] = []
  for (let index = start; index < end; index += 1) {
    const unit = units[index]
    if (unit === undefined) continue
    if ('token' in unit) {
      const token = unit.token
      if (token.kind === 'details-open' && token.pair !== undefined) {
        const closeIndex = token.pairIndex ?? -1
        const summaryStart = token.summaryOpen
        const summaryEnd = token.summaryClose
        if (
          closeIndex < 0 ||
          summaryStart === undefined ||
          summaryEnd === undefined
        ) {
          continue
        }
        const summaryStartIndex = token.summaryOpenIndex ?? -1
        const summaryEndIndex = token.summaryCloseIndex ?? -1
        const summaryUnits = units.slice(summaryStartIndex + 1, summaryEndIndex)
        const summaryText = summaryUnits
          .map((summaryUnit) =>
            'token' in summaryUnit ? summaryUnit.token.value : '',
          )
          .join('')
        const openAttribute = /^<details\s+open(?:\s*=|\s*>)/i.test(token.value)
        const children = renderRange(
          units,
          summaryEndIndex + 1,
          closeIndex,
          parse,
          references,
          true,
        )
        result.push({
          type: 'details',
          children: [
            parseSummary(
              units,
              summaryStartIndex + 1,
              summaryEndIndex,
              summaryText,
              parse,
              references,
            ),
            ...children,
          ],
          data: {
            hName: 'details',
            ...(openAttribute ? { hProperties: { open: true } } : {}),
          },
        })
        index = closeIndex
        continue
      }
      if (token.kind === 'text') {
        const base = token.group.node.position?.start.offset ?? 0
        if (markdownContext || token.afterMatchedClose === true) {
          result.push(
            ...markdownNodes(
              token.value,
              base + token.offset,
              parse,
              references,
            ),
          )
        } else {
          result.push({ ...token.group.node, value: token.value })
        }
      } else if (token.matched !== true) {
        result.push({ ...token.group.node, value: token.value })
      }
      continue
    }
    result.push(unit.node)
  }
  return result
}

function transformNodes(
  nodes: MarkdownNode[],
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
): MarkdownNode[] {
  return renderUnits(
    toUnits(nodes, parse, references),
    parse,
    references,
    false,
  )
}

export function remarkMarkdownDetails(this: MarkdownProcessor) {
  const parse: MarkdownParser = (value) => {
    const tree = this.parse(value)
    return isRoot(tree) ? tree : { type: 'root', children: [] }
  }
  return (tree: MarkdownRoot, file: { value: string | Uint8Array }) => {
    const source =
      typeof file.value === 'string'
        ? file.value
        : new TextDecoder().decode(file.value)
    const references = getReferenceDefinitionSources(tree.children, source)
    tree.children = transformNodes(tree.children, parse, references)
  }
}

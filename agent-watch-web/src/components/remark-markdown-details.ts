import type {
  MarkdownNode,
  MarkdownRoot,
} from '#components/markdown-message-types'
import {
  getReferenceDefinitionSources,
  type MarkdownParser,
  parseMarkdownSection,
  type ReferenceDefinitionSource,
} from '#components/remark-markdown-details/source'

const htmlCandidateStart = /^\s*<\/?(?:details|summary)\b/i
const htmlLexemePattern =
  /<!--[\s\S]*?(?:-->|$)|<(script|style|pre|textarea)\b[\s\S]*?(?:<\/\1\s*>|$)|<\/?(?:details|summary)\b(?:[^>"']|"[^"]*"|'[^']*')*>/gi
const detailsOpenPattern =
  /^<details(?:\s+open(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)?\s*>$/i
const detailsClosePattern = /^<\/details\s*>$/i
const summaryOpenPattern = /^<summary\s*>$/i
const summaryClosePattern = /^<\/summary\s*>$/i

type Processor = { parse: (value: string) => unknown }
type Kind =
  | 'details-open'
  | 'details-raw-open'
  | 'details-close'
  | 'summary-open'
  | 'summary-close'
  | 'summary-text'
type Marker = { kind: Kind; value: string; node: MarkdownNode; offset: number }
type HtmlGroup = { node: MarkdownNode; offset: number }
type Unit =
  | { node: MarkdownNode; group?: HtmlGroup }
  | { marker: Marker; group?: HtmlGroup }
type Pair = { close: number; summaryOpen: number; summaryClose: number }

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

function marker(
  kind: Kind,
  value: string,
  node: MarkdownNode,
  offset: number,
  group?: HtmlGroup,
): Unit {
  return { marker: { kind, value, node, offset }, ...(group ? { group } : {}) }
}

function section(
  source: string,
  offset: number,
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
) {
  return source.trim() === ''
    ? []
    : parseMarkdownSection(source, parse, references, offset)
}

function splitHtml(
  node: MarkdownNode,
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
): Unit[] | null {
  if (
    node.type !== 'html' ||
    node.value === undefined ||
    !htmlCandidateStart.test(node.value)
  ) {
    return null
  }
  const base = node.position?.start.offset ?? 0
  const group = { node, offset: base }
  const units: Unit[] = []
  const pattern = new RegExp(htmlLexemePattern.source, 'gi')
  let cursor = 0
  let inSummary = false
  const appendText = (end: number) => {
    const text = node.value?.slice(cursor, end) ?? ''
    if (text !== '') {
      if (inSummary) {
        units.push(marker('summary-text', text, node, base + cursor, group))
      } else {
        for (const parsed of section(text, base + cursor, parse, references)) {
          units.push({ node: parsed, group })
        }
      }
    }
  }

  for (const match of node.value.matchAll(pattern)) {
    const at = match.index
    const value = match[0]
    appendText(at)
    if (/^<!--|^<(?:script|style|pre|textarea)\b/i.test(value)) {
      units.push({ node: { ...node, value }, group })
    } else {
      const kind: Kind | null = detailsOpenPattern.test(value)
        ? 'details-open'
        : detailsClosePattern.test(value)
          ? 'details-close'
          : summaryOpenPattern.test(value)
            ? 'summary-open'
            : summaryClosePattern.test(value)
              ? 'summary-close'
              : /^<details\b/i.test(value)
                ? 'details-raw-open'
                : null
      if (kind === null) units.push({ node: { ...node, value }, group })
      else {
        units.push(marker(kind, value, node, base + at, group))
        if (kind === 'summary-open') inSummary = true
        if (kind === 'summary-close') inSummary = false
      }
    }
    cursor = at + value.length
  }
  appendText(node.value.length)
  return units
}

function unitsFrom(
  nodes: MarkdownNode[],
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
): Unit[] {
  const units: Unit[] = []
  for (const node of nodes) {
    const split = splitHtml(node, parse, references)
    if (split === null) units.push({ node })
    else for (const unit of split) units.push(unit)
  }
  return units
}

function pairDetails(units: Unit[]) {
  type Frame = {
    open: number
    render: boolean
    summaryOpen?: number
    summaryClose?: number
  }
  const pairs = new Map<number, Pair>()
  const closes = new Map<number, number>()
  const stack: Frame[] = []
  const pending = new Set<Frame>()
  const invalidatePending = () => {
    pending.clear()
  }

  for (const [index, unit] of units.entries()) {
    if (!('marker' in unit)) {
      invalidatePending()
      continue
    }
    const { kind } = unit.marker
    if (kind === 'details-open') {
      invalidatePending()
      const frame = { open: index, render: true }
      stack.push(frame)
      pending.add(frame)
    } else if (kind === 'details-raw-open') {
      invalidatePending()
      stack.push({ open: index, render: false })
    } else if (kind === 'summary-open') {
      const frame = stack.at(-1)
      if (frame?.render === true && pending.delete(frame))
        frame.summaryOpen = index
      else invalidatePending()
    } else if (kind === 'summary-close') {
      const frame = stack.at(-1)
      if (
        frame?.render === true &&
        frame.summaryOpen !== undefined &&
        frame.summaryClose === undefined
      ) {
        frame.summaryClose = index
      } else invalidatePending()
    } else if (kind === 'details-close') {
      const frame = stack.pop()
      if (frame === undefined) continue
      pending.delete(frame)
      closes.set(frame.open, index)
      if (
        frame.render &&
        frame.summaryOpen !== undefined &&
        frame.summaryClose !== undefined
      ) {
        pairs.set(frame.open, {
          close: index,
          summaryOpen: frame.summaryOpen,
          summaryClose: frame.summaryClose,
        })
      }
    }
  }
  return { pairs, closes }
}

function rawDetailsNode(group: HtmlGroup, open: Marker, close: Marker) {
  const value = group.node.value ?? ''
  const start = open.offset - group.offset
  const end = close.offset - group.offset + close.value.length
  const position = group.node.position
  return {
    ...group.node,
    value: value.slice(start, end),
    ...(position === undefined
      ? {}
      : {
          position: {
            ...position,
            start: { ...position.start, offset: open.offset },
            end: { ...position.end, offset: close.offset + close.value.length },
          },
        }),
  }
}

function summaryNode(
  units: Unit[],
  start: number,
  end: number,
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
): MarkdownNode {
  const children: MarkdownNode[] = []
  let fallback = ''
  const append = (nodes: MarkdownNode[]) => {
    for (const node of nodes) {
      if (node.type !== 'paragraph') continue
      if (children.length > 0) children.push({ type: 'break' })
      for (const child of node.children ?? []) children.push(child)
    }
  }
  for (let index = start; index < end; index += 1) {
    const unit = units[index]
    if (unit === undefined) continue
    if ('marker' in unit && unit.marker.kind === 'summary-text') {
      fallback += unit.marker.value
      append(section(unit.marker.value, unit.marker.offset, parse, references))
    } else if (!('marker' in unit) && unit.node.type === 'paragraph') {
      append([unit.node])
      fallback += (unit.node.children ?? [])
        .filter((child) => child.type === 'text')
        .map((child) => child.value ?? '')
        .join('')
    } else if (!('marker' in unit) && unit.node.type === 'html') {
      fallback += unit.node.value ?? ''
    }
  }
  return {
    type: 'summary',
    children:
      children.length === 0 ? [{ type: 'text', value: fallback }] : children,
    data: {
      hName: 'span',
      hProperties: { className: 'markdown-details-summary-content' },
    },
  }
}

function toNode(unit: Unit): MarkdownNode {
  if (!('marker' in unit)) return unit.node
  if (unit.marker.kind === 'summary-text') {
    return { type: 'text', value: unit.marker.value }
  }
  return { ...unit.marker.node, type: 'html', value: unit.marker.value }
}

function renderUnits(
  nodes: MarkdownNode[],
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
): MarkdownNode[] {
  const units = unitsFrom(nodes, parse, references)
  const { pairs, closes } = pairDetails(units)
  const result: MarkdownNode[] = []
  const frames: Array<{
    close: number
    node: MarkdownNode
    parent: MarkdownNode[]
  }> = []
  let output = result

  for (let index = 0; index < units.length; index += 1) {
    const unit = units[index]
    if (unit === undefined) continue
    if (
      'marker' in unit &&
      (unit.marker.kind === 'details-open' ||
        unit.marker.kind === 'details-raw-open')
    ) {
      const pair =
        unit.marker.kind === 'details-open' ? pairs.get(index) : undefined
      if (pair !== undefined) {
        const children = [
          summaryNode(
            units,
            pair.summaryOpen + 1,
            pair.summaryClose,
            parse,
            references,
          ),
        ]
        const node: MarkdownNode = {
          type: 'details',
          children,
          data: {
            hName: 'details',
            ...(/^<details\s+open(?:\s*=|\s*>)/i.test(unit.marker.value)
              ? { hProperties: { open: true } }
              : {}),
          },
        }
        frames.push({
          close: pair.close,
          node,
          parent: output,
        })
        output = children
        index = pair.summaryClose
        continue
      }
      if (unit.group !== undefined) {
        const closeIndex = closes.get(index)
        const closeUnit =
          closeIndex === undefined ? undefined : units[closeIndex]
        if (
          closeUnit !== undefined &&
          'marker' in closeUnit &&
          closeUnit.group === unit.group
        ) {
          output.push(rawDetailsNode(unit.group, unit.marker, closeUnit.marker))
          index = closeIndex
          continue
        }
        output.push(unit.group.node)
        while (units[index + 1]?.group === unit.group) index += 1
        continue
      }
    }

    if (
      'marker' in unit &&
      unit.marker.kind === 'details-close' &&
      frames.at(-1)?.close === index
    ) {
      const frame = frames.pop()
      if (frame !== undefined) {
        output = frame.parent
        output.push(frame.node)
      }
    } else if ('marker' in unit) {
      output.push(toNode(unit))
    } else {
      if (unit.node.children !== undefined) {
        unit.node.children = renderUnits(unit.node.children, parse, references)
      }
      output.push(unit.node)
    }
  }
  return result
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
    const references = getReferenceDefinitionSources(tree.children, source)
    tree.children = renderUnits(tree.children, parse, references)
  }
}

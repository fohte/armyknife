import type { MarkdownNode } from '#components/markdown-message-types'
import {
  type HtmlSourceAnalysis,
  sourceOffsetsForRange,
} from '#components/remark-markdown-details/html-source'
import {
  type MarkdownParser,
  parseMarkdownSection,
  type ReferenceDefinitionSource,
  type SourceOffsetMap,
} from '#components/remark-markdown-details/source'

const detailsOpenPattern =
  /^<details(?:\s+open(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)?\s*>$/i
const detailsClosePattern = /^<\/details\s*>$/i
const summaryOpenPattern = /^<summary\s*>$/i
const summaryClosePattern = /^<\/summary\s*>$/i

type Kind =
  | 'details-open'
  | 'details-raw-open'
  | 'details-close'
  | 'summary-open'
  | 'summary-close'
  | 'summary-text'
type Marker = {
  kind: Kind
  value: string
  node: MarkdownNode
  offset: number
  localOffset: number
}
type HtmlGroup = HtmlSourceAnalysis['group']
type Unit =
  | { node: MarkdownNode; group?: HtmlGroup }
  | { marker: Marker; group?: HtmlGroup }
type Pair = { close: number; summaryOpen: number; summaryClose: number }
function marker(
  kind: Kind,
  value: string,
  node: MarkdownNode,
  localOffset: number,
  group: HtmlGroup,
): Unit {
  return {
    marker: {
      kind,
      value,
      node,
      localOffset,
      offset: group.offsets[localOffset] ?? group.offset + localOffset,
    },
    group,
  }
}

function section(
  source: string,
  offset: number,
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
  sourceOffsets?: SourceOffsetMap,
) {
  return source.trim() === ''
    ? []
    : parseMarkdownSection(source, parse, references, offset, sourceOffsets)
}

function splitHtml(
  node: MarkdownNode,
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
  analysis: HtmlSourceAnalysis | undefined,
): Unit[] | null {
  if (
    node.type !== 'html' ||
    node.value === undefined ||
    analysis === undefined
  ) {
    return null
  }
  const { codeRanges, group, lexemes } = analysis
  const units: Unit[] = []
  let cursor = 0
  let codeRangeIndex = 0
  let inSummary = false
  const appendText = (end: number) => {
    const text = node.value?.slice(cursor, end) ?? ''
    if (text !== '') {
      const sourceOffsets = sourceOffsetsForRange(group, cursor, end)
      const offset = sourceOffsets[0] ?? group.offset + cursor
      if (inSummary) {
        units.push(marker('summary-text', text, node, cursor, group))
      } else {
        for (const parsed of section(
          text,
          offset,
          parse,
          references,
          sourceOffsets,
        )) {
          units.push({ node: parsed, group })
        }
      }
    }
  }

  for (const { at, value } of lexemes) {
    while (
      codeRanges[codeRangeIndex] !== undefined &&
      codeRanges[codeRangeIndex].end <= at
    ) {
      codeRangeIndex += 1
    }
    const codeRange = codeRanges[codeRangeIndex]
    if (
      codeRange !== undefined &&
      codeRange.start <= at &&
      at < codeRange.end
    ) {
      continue
    }
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
        units.push(marker(kind, value, node, at, group))
        if (kind === 'summary-open') inSummary = true
        if (kind === 'summary-close') inSummary = false
      }
    }
    cursor = at + value.length
  }
  appendText((node.value ?? '').length)
  return units
}

function unitsFrom(
  nodes: MarkdownNode[],
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
  analyses: ReadonlyMap<MarkdownNode, HtmlSourceAnalysis>,
): Unit[] {
  const units: Unit[] = []
  for (const node of nodes) {
    const split = splitHtml(node, parse, references, analyses.get(node))
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

function rawDetailsNode(group: HtmlGroup, open: Marker, end: number) {
  const start = open.localOffset
  const startOffset = group.offsets[start] ?? group.offset + start
  const endOffset = group.offsets[end] ?? group.offset + end
  const position = group.node.position
  return {
    ...group.node,
    value: (group.node.value ?? '').slice(start, end),
    ...(position === undefined
      ? {}
      : {
          position: {
            ...position,
            start: { ...position.start, offset: startOffset },
            end: { ...position.end, offset: endOffset },
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
  let hasParagraph = false
  const append = (nodes: MarkdownNode[], separateFirstParagraph: boolean) => {
    let appendedParagraph = false
    for (const node of nodes) {
      if (node.type !== 'paragraph') continue
      if (appendedParagraph || (hasParagraph && separateFirstParagraph)) {
        children.push({ type: 'break' })
      }
      for (const child of node.children ?? []) children.push(child)
      appendedParagraph = true
      hasParagraph = true
    }
  }
  for (let index = start; index < end; index += 1) {
    const unit = units[index]
    if (unit === undefined) continue
    if ('marker' in unit && unit.marker.kind === 'summary-text') {
      fallback += unit.marker.value
      const sourceOffsets =
        unit.group === undefined
          ? undefined
          : sourceOffsetsForRange(
              unit.group,
              unit.marker.localOffset,
              unit.marker.localOffset + unit.marker.value.length,
            )
      append(
        section(
          unit.marker.value,
          unit.marker.offset,
          parse,
          references,
          sourceOffsets,
        ),
        /^[ \t]*\r?\n[ \t]*\r?\n/.test(unit.marker.value),
      )
    } else if (!('marker' in unit) && unit.node.type === 'paragraph') {
      append([unit.node], true)
      fallback += (unit.node.children ?? [])
        .filter((child) => child.type === 'text')
        .map((child) => child.value ?? '')
        .join('')
    } else if (!('marker' in unit) && unit.node.type === 'html') {
      const value = unit.node.value ?? ''
      children.push({ type: 'text', value })
      fallback += value
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
  analyses: ReadonlyMap<MarkdownNode, HtmlSourceAnalysis>,
): MarkdownNode[] {
  const units = unitsFrom(nodes, parse, references, analyses)
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
        const ownCloseIndex = closes.get(index)
        const ownCloseUnit =
          ownCloseIndex === undefined ? undefined : units[ownCloseIndex]
        const ownClose =
          ownCloseUnit !== undefined &&
          'marker' in ownCloseUnit &&
          ownCloseUnit.group === unit.group
            ? ownCloseUnit.marker
            : undefined
        const activeCloseIndex = frames.at(-1)?.close
        const activeCloseUnit =
          activeCloseIndex === undefined ? undefined : units[activeCloseIndex]
        const activeClose =
          activeCloseUnit !== undefined &&
          'marker' in activeCloseUnit &&
          activeCloseUnit.group === unit.group
            ? activeCloseUnit.marker
            : undefined
        const end =
          ownClose !== undefined
            ? ownClose.localOffset + ownClose.value.length
            : activeClose !== undefined
              ? activeClose.localOffset
              : (unit.group.node.value ?? '').length
        output.push(rawDetailsNode(unit.group, unit.marker, end))
        if (ownClose !== undefined && ownCloseIndex !== undefined) {
          index = ownCloseIndex
        } else if (
          activeClose !== undefined &&
          activeCloseIndex !== undefined
        ) {
          index = activeCloseIndex - 1
        } else {
          while (units[index + 1]?.group === unit.group) index += 1
        }
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
        unit.node.children = renderUnits(
          unit.node.children,
          parse,
          references,
          analyses,
        )
      }
      output.push(unit.node)
    }
  }
  return result
}

export function getSupportedDetailsIntervals(
  nodes: MarkdownNode[],
  parse: MarkdownParser,
  analyses: ReadonlyMap<MarkdownNode, HtmlSourceAnalysis>,
) {
  const units = unitsFrom(nodes, parse, [], analyses)
  const { pairs } = pairDetails(units)
  const intervals: Array<{ end: number; start: number }> = []
  for (let openIndex = 0; openIndex < units.length; openIndex += 1) {
    const pair = pairs.get(openIndex)
    if (pair === undefined) continue
    const open = units[openIndex]
    const close = units[pair.close]
    if (
      open !== undefined &&
      'marker' in open &&
      close !== undefined &&
      'marker' in close
    ) {
      intervals.push({ start: open.marker.offset, end: close.marker.offset })
    }
  }
  return intervals
}

export function renderMarkdownDetails(
  nodes: MarkdownNode[],
  parse: MarkdownParser,
  references: readonly ReferenceDefinitionSource[],
  analyses: ReadonlyMap<MarkdownNode, HtmlSourceAnalysis>,
) {
  return renderUnits(nodes, parse, references, analyses)
}

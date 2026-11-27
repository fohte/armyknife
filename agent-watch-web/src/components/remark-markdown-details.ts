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

const controlTag =
  /^(<details(?:\s+open(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?)?\s*>|<\/details\s*>|<summary\s*>|<\/summary\s*>)/i
const inlineSummary = /^\s*<summary\s*>([\s\S]*?)<\/summary\s*>/i
const maxDetailsNesting = 32

type Processor = { parse: (value: string) => unknown }
type Kind =
  | 'details-open'
  | 'details-close'
  | 'summary-open'
  | 'summary-close'
  | 'summary-text'
type Marker = { kind: Kind; value: string; node: MarkdownNode; offset: number }
type Unit = { node: MarkdownNode } | { marker: Marker }
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
): Unit {
  return { marker: { kind, value, node, offset } }
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
  if (node.type !== 'html' || node.value === undefined) return null
  const leading = node.value.length - node.value.trimStart().length
  const raw = node.value.slice(leading)
  const base = (node.position?.start.offset ?? 0) + leading
  const units: Unit[] = []
  let consumed = 0
  const add = (kind: Kind, value: string, at: number) => {
    units.push(marker(kind, value, node, base + at))
  }

  const tag = controlTag.exec(raw)?.[0]
  if (tag === undefined) return null
  const kind: Kind = /^<details/i.test(tag)
    ? 'details-open'
    : /^<\/details/i.test(tag)
      ? 'details-close'
      : /^<summary/i.test(tag)
        ? 'summary-open'
        : 'summary-close'
  add(kind, tag, 0)
  consumed = tag.length

  if (kind === 'details-open') {
    const inline = inlineSummary.exec(raw.slice(consumed))
    if (inline !== null) {
      const start = consumed + inline[0].length - inline[0].trimStart().length
      add('summary-open', '<summary>', start)
      add('summary-text', inline[1] ?? '', start + '<summary>'.length)
      add(
        'summary-close',
        '</summary>',
        start + inline[0].length - '</summary>'.length,
      )
      consumed += inline[0].length
    }
  } else if (kind === 'summary-open') {
    const inner = /^[\s\S]*?<\/summary\s*>/i.exec(raw.slice(consumed))
    if (inner !== null) {
      const endTag = /<\/summary\s*>$/i.exec(inner[0])?.[0] ?? '</summary>'
      add('summary-text', inner[0].slice(0, -endTag.length), consumed)
      add('summary-close', endTag, consumed + inner[0].length - endTag.length)
      consumed += inner[0].length
    }
  }

  for (const unit of unitsFrom(
    section(
      node.value.slice(leading + consumed),
      base + consumed,
      parse,
      references,
    ),
    parse,
    references,
  )) {
    units.push(unit)
  }
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

function pairDetails(units: Unit[]): Map<number, Pair> {
  type Frame = { open: number; summaryOpen?: number; summaryClose?: number }
  const pairs = new Map<number, Pair>()
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
      const frame = { open: index }
      stack.push(frame)
      pending.add(frame)
    } else if (kind === 'summary-open') {
      const frame = stack.at(-1)
      if (frame !== undefined && pending.delete(frame))
        frame.summaryOpen = index
      else invalidatePending()
    } else if (kind === 'summary-close') {
      const frame = stack.at(-1)
      if (
        frame?.summaryOpen !== undefined &&
        frame.summaryClose === undefined
      ) {
        frame.summaryClose = index
      } else invalidatePending()
    } else if (kind === 'details-close') {
      const frame = stack.pop()
      if (frame === undefined) continue
      pending.delete(frame)
      if (frame.summaryOpen !== undefined && frame.summaryClose !== undefined) {
        pairs.set(frame.open, {
          close: index,
          summaryOpen: frame.summaryOpen,
          summaryClose: frame.summaryClose,
        })
      }
    }
  }
  return pairs
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
  depth: number,
): MarkdownNode[] {
  const units = unitsFrom(nodes, parse, references)
  const pairs = pairDetails(units)
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
    if ('marker' in unit && unit.marker.kind === 'details-open') {
      const pair = pairs.get(index)
      if (pair !== undefined) {
        if (depth + frames.length >= maxDetailsNesting) {
          for (let raw = index; raw <= pair.close; raw += 1) {
            const rawUnit = units[raw]
            if (rawUnit !== undefined) output.push(toNode(rawUnit))
          }
          index = pair.close
          continue
        }
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
        frames.push({ close: pair.close, node, parent: output })
        output = children
        index = pair.summaryClose
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
          depth + frames.length,
        )
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
    tree.children = renderUnits(tree.children, parse, references, 0)
  }
}

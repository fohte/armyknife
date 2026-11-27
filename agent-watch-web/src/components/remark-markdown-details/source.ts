import type {
  MarkdownNode,
  MarkdownRoot,
} from '#components/markdown-message-types'

export type MarkdownParser = (value: string) => MarkdownRoot
export type ReferenceDefinitionSource = { offset: number; source: string }

function shiftMarkdownNodeOffsets(nodes: MarkdownNode[], offset: number) {
  const pending = [...nodes]
  while (pending.length > 0) {
    const node = pending.pop()
    if (node === undefined) continue
    if (node.position !== undefined) {
      const { start, end } = node.position
      node.position = {
        start: {
          ...start,
          ...(start.offset === undefined
            ? {}
            : { offset: start.offset + offset }),
        },
        end: {
          ...end,
          ...(end.offset === undefined ? {} : { offset: end.offset + offset }),
        },
      }
    }
    if (node.children !== undefined) {
      for (const child of node.children) pending.push(child)
    }
  }
}

export function getReferenceDefinitionSources(
  nodes: readonly MarkdownNode[],
  source: string,
): ReferenceDefinitionSource[] {
  const pending = [...nodes].reverse()
  const definitions: ReferenceDefinitionSource[] = []
  while (pending.length > 0) {
    const node = pending.pop()
    if (node === undefined) continue
    if (node.type === 'definition') {
      const start = node.position?.start.offset
      const end = node.position?.end.offset
      if (start !== undefined && end !== undefined) {
        definitions.push({ offset: start, source: source.slice(start, end) })
      }
    }
    if (node.children !== undefined) {
      for (let index = node.children.length - 1; index >= 0; index -= 1) {
        const child = node.children[index]
        if (child !== undefined) pending.push(child)
      }
    }
  }
  return definitions
}

export function parseMarkdownSection(
  source: string,
  parse: MarkdownParser,
  definitions: readonly ReferenceDefinitionSource[],
  sourceOffset: number,
): MarkdownNode[] {
  const preceding = definitions
    .filter((definition) => definition.offset < sourceOffset)
    .map((definition) => definition.source)
  const following = definitions
    .filter((definition) => definition.offset >= sourceOffset + source.length)
    .map((definition) => definition.source)
  const prefix = preceding.length > 0 ? `${preceding.join('\n\n')}\n\n` : ''
  const suffix = following.length > 0 ? `\n\n${following.join('\n\n')}` : ''
  const sectionStart = prefix.length
  const sectionEnd = sectionStart + source.length
  const tree = parse(`${prefix}${source}${suffix}`)
  tree.children = tree.children.filter((node) => {
    const start = node.position?.start.offset
    return start === undefined || (start >= sectionStart && start < sectionEnd)
  })
  shiftMarkdownNodeOffsets(tree.children, sourceOffset - sectionStart)
  return tree.children
}

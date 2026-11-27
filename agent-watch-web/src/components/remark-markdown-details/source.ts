import type {
  MarkdownNode,
  MarkdownRoot,
} from '#components/markdown-message-types'

export type MarkdownParser = (value: string) => MarkdownRoot

export type ReferenceDefinitionSource = {
  offset: number
  source: string
}

export function shiftMarkdownNodeOffsets(
  nodes: MarkdownNode[],
  offset: number,
) {
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
    if (node.children !== undefined) pending.push(...node.children)
  }
}

export function getReferenceDefinitionSources(
  nodes: readonly MarkdownNode[],
  source: string,
): ReferenceDefinitionSource[] {
  const definitions: ReferenceDefinitionSource[] = []
  const pending: MarkdownNode[] = []
  for (let index = nodes.length - 1; index >= 0; index -= 1) {
    const node = nodes[index]
    if (node !== undefined) pending.push(node)
  }
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
  referenceDefinitions: readonly ReferenceDefinitionSource[],
  sourceOffset: number,
) {
  const precedingDefinitions: string[] = []
  const followingDefinitions: string[] = []
  for (const definition of referenceDefinitions) {
    if (definition.offset < sourceOffset)
      precedingDefinitions.push(definition.source)
    else if (definition.offset >= sourceOffset + source.length) {
      followingDefinitions.push(definition.source)
    }
  }
  const prefix =
    precedingDefinitions.length === 0
      ? ''
      : `${precedingDefinitions.join('\n\n')}\n\n`
  const suffix =
    followingDefinitions.length === 0
      ? ''
      : `\n\n${followingDefinitions.join('\n\n')}`
  const sectionStart = prefix.length
  const sectionEnd = sectionStart + source.length
  const tree = parse(`${prefix}${source}${suffix}`)
  tree.children = tree.children.filter((node) => {
    const start = node.position?.start.offset
    return start === undefined || (start >= sectionStart && start < sectionEnd)
  })
  shiftMarkdownNodeOffsets(tree.children, -sectionStart)
  return { source, tree }
}

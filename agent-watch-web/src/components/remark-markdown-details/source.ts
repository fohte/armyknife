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
  for (const node of nodes) {
    if (node.position !== undefined) {
      const start = node.position.start.offset
      const end = node.position.end.offset
      node.position = {
        start: {
          ...node.position.start,
          ...(start === undefined ? {} : { offset: start + offset }),
        },
        end: {
          ...node.position.end,
          ...(end === undefined ? {} : { offset: end + offset }),
        },
      }
    }
    if (node.children !== undefined) {
      shiftMarkdownNodeOffsets(node.children, offset)
    }
  }
}

export function getReferenceDefinitionSources(
  nodes: readonly MarkdownNode[],
  source: string,
): ReferenceDefinitionSource[] {
  const definitions: ReferenceDefinitionSource[] = []
  for (const node of nodes) {
    if (node.type === 'definition') {
      const start = node.position?.start.offset
      const end = node.position?.end.offset
      if (start !== undefined && end !== undefined) {
        definitions.push({ offset: start, source: source.slice(start, end) })
      }
    }
    if (node.children !== undefined) {
      definitions.push(...getReferenceDefinitionSources(node.children, source))
    }
  }
  return definitions.sort((left, right) => left.offset - right.offset)
}

export function parseMarkdownSection(
  source: string,
  parse: MarkdownParser,
  referenceDefinitions: readonly ReferenceDefinitionSource[],
  sourceOffset: number,
) {
  const precedingDefinitions = referenceDefinitions.filter(
    (definition) => definition.offset < sourceOffset,
  )
  const followingDefinitions = referenceDefinitions.filter(
    (definition) => definition.offset >= sourceOffset + source.length,
  )
  const prefix =
    precedingDefinitions.length === 0
      ? ''
      : `${precedingDefinitions.map(({ source }) => source).join('\n\n')}\n\n`
  const suffix =
    followingDefinitions.length === 0
      ? ''
      : `\n\n${followingDefinitions.map(({ source }) => source).join('\n\n')}`
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

export function trimBlankLines(source: string) {
  const leadingBlankLines = /^(?:[\t ]*\r?\n)+/.exec(source)?.[0] ?? ''
  const trimmedSource = source
    .slice(leadingBlankLines.length)
    .replace(/(?:\r?\n[\t ]*)+$/, '')
  return { offset: leadingBlankLines.length, source: trimmedSource }
}

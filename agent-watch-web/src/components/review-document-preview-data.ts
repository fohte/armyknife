type ReviewCommentPreview = {
  author: string
  createdAt: string
  body: string
}

type ReviewDiffLine = {
  text: string
  kind: 'added' | 'removed' | 'context' | 'marker'
  focused: boolean
}

type ReviewThreadPreview = {
  id: string
  path: string
  line: number | null
  resolved: boolean
  diff: ReviewDiffLine[]
  comments: ReviewCommentPreview[]
  reply: string
}

type IssueCommentPreview = {
  body: string
}

export function parseIssueComment(body: string): IssueCommentPreview {
  const lines = body.split('\n')
  const metadataKeys = new Set([
    'author',
    'createdAt',
    'updatedAt',
    'id',
    'databaseId',
  ])
  let headerLength = 0
  for (const line of lines) {
    const match = /^<!-- ([^:]+): (.*) -->$/.exec(line)
    if (match === null || !metadataKeys.has(match[1] ?? '')) break
    headerLength += 1
  }
  const bodyStart =
    headerLength === 0
      ? lines.findIndex((line) => line.length > 0)
      : lines[headerLength] === ''
        ? headerLength + 1
        : headerLength

  return {
    body: lines.slice(bodyStart < 0 ? lines.length : bodyStart).join('\n'),
  }
}

export function parseReviewThreads(body: string): ReviewThreadPreview[] {
  return body
    .split(/(?=^<!-- thread: )/m)
    .filter((block) => block.startsWith('<!-- thread: '))
    .map((block) => parseReviewThread(block))
    .filter((thread): thread is ReviewThreadPreview => thread !== null)
}

function parseReviewThread(block: string): ReviewThreadPreview | null {
  const header =
    /^<!-- thread: (\S+) path: (.+?)(?::(\d+))?(?: author: @\S+)? -->/m.exec(
      block,
    )
  if (header === null) return null

  const pathWithLine = header[2] ?? ''
  const pathLine = /^(.*):(\d+)$/.exec(pathWithLine)
  const path = pathLine?.[1] ?? pathWithLine
  const line = Number(pathLine?.[2] ?? header[3])
  const diffMatch =
    /<!-- diff -->\s*```diff\r?\n([\s\S]*?)\r?\n```\s*<!-- \/diff -->/.exec(
      block,
    )
  const comments = [
    ...block.matchAll(
      /<!-- comment: (@\S+) (\S+) -->\r?\n([\s\S]*?)\r?\n<!-- \/comment -->/g,
    ),
  ].map(([, author, createdAt, text]) => ({
    author: author ?? 'unknown',
    createdAt: createdAt ?? '',
    body: (text ?? '').replace(/\r?\n$/, ''),
  }))
  const lastCommentEnd = block.lastIndexOf('<!-- /comment -->')
  const replyStart =
    lastCommentEnd >= 0
      ? lastCommentEnd + '<!-- /comment -->'.length
      : diffMatch === null
        ? header[0].length
        : diffMatch.index + diffMatch[0].length
  const reply = block.slice(replyStart).trim()
  const diff = diffMatch?.[1] ?? ''

  return {
    id: header[1] ?? '',
    path,
    line: Number.isSafeInteger(line) && line > 0 ? line : null,
    resolved: /^- \[x\] resolve$/m.test(block),
    diff: parseDiffLines(diff, Number.isSafeInteger(line) ? line : null),
    comments,
    reply,
  }
}

function parseDiffLines(
  diff: string,
  focusLine: number | null,
): ReviewDiffLine[] {
  const lines = diff.split('\n')
  const hasExplicitFocus = lines.some((line) => line.startsWith('>'))
  let newLine: number | null = null
  return lines.map((text) => {
    const focused = text.startsWith('>')
    const displayText = focused ? text.slice(1) : text
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(displayText)
    if (hunk !== null) {
      newLine = Number(hunk[1])
      return { text: displayText, kind: 'marker', focused: false }
    }
    if (displayText.startsWith('+') && !displayText.startsWith('+++')) {
      const lineFocused =
        focused || (!hasExplicitFocus && newLine === focusLine)
      if (newLine !== null) newLine += 1
      return { text: displayText, kind: 'added', focused: lineFocused }
    }
    if (displayText.startsWith('-') && !displayText.startsWith('---')) {
      return { text: displayText, kind: 'removed', focused }
    }
    if (displayText.startsWith(' ') || displayText.length === 0) {
      const lineFocused =
        focused || (!hasExplicitFocus && newLine === focusLine)
      if (newLine !== null) newLine += 1
      return { text: displayText, kind: 'context', focused: lineFocused }
    }
    return { text: displayText, kind: 'marker', focused }
  })
}

export function updateReviewThread(
  body: string,
  threadId: string,
  changes: { resolved?: boolean; reply?: string },
): string {
  const blocks = body.split(/(?=^<!-- thread: )/m)
  return blocks
    .map((block) => {
      const thread = parseReviewThread(block)
      if (thread?.id !== threadId) return block
      let updated = block
      if (changes.resolved !== undefined) {
        updated = updated.replace(
          /^- \[[ x]\] resolve$/m,
          `- [${changes.resolved ? 'x' : ' '}] resolve`,
        )
      }
      if (changes.reply !== undefined) {
        const parsed = parseReviewThread(updated)
        if (parsed === null) return updated
        const commentEnd = updated.lastIndexOf('<!-- /comment -->')
        const diffMatch =
          /<!-- diff -->\s*```diff\r?\n[\s\S]*?\r?\n```\s*<!-- \/diff -->/.exec(
            updated,
          )
        const trailerStart =
          commentEnd >= 0
            ? commentEnd + '<!-- /comment -->'.length
            : diffMatch === null
              ? 0
              : diffMatch.index + diffMatch[0].length
        const prefix = updated.slice(0, trailerStart).replace(/\s*$/, '')
        const trailing = /\n*$/.exec(updated)?.[0] ?? ''
        updated =
          changes.reply.length === 0
            ? `${prefix}${trailing}`
            : `${prefix}\n${changes.reply}${trailing}`
      }
      return updated
    })
    .join('')
}

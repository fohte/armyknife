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

const LEGACY_LINES_BEFORE = 5

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

export function parseReviewMessages(body: string): string[] {
  const section = /<!-- reviews -->\r?\n([\s\S]*?)\r?\n<!-- \/reviews -->/.exec(
    body,
  )?.[1]
  if (section === undefined) return []

  return [
    ...section.matchAll(
      /^<!-- review: @\S+ state=\S+ \S+ -->\r?\n([\s\S]*?)\r?\n<!-- \/review -->/gm,
    ),
  ].map(([, message]) => (message ?? '').replace(/\r?\n$/, ''))
}

export function parseReviewThreads(body: string): ReviewThreadPreview[] {
  const blocks = body
    .split(/(?=^<!-- thread: )/m)
    .filter((block) => block.startsWith('<!-- thread: '))
  return blocks
    .map((block, index) =>
      parseReviewThread(
        index < blocks.length - 1 ? stripLineEnding(block, 'end') : block,
      ),
    )
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
      /<!-- comment: (@\S+) (\S+) -->\r?\n([\s\S]*?)(?:\r?\n)?<!-- \/comment -->/g,
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
  const reply = stripLineEnding(block.slice(replyStart), 'both')
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

function stripLineEnding(value: string, position: 'start' | 'end' | 'both') {
  const withoutStart =
    position === 'start' || position === 'both'
      ? value.replace(/^\r?\n/, '')
      : value
  return position === 'end' || position === 'both'
    ? withoutStart.replace(/\r?\n$/, '')
    : withoutStart
}

function parseDiffLines(
  diff: string,
  focusLine: number | null,
): ReviewDiffLine[] {
  const lines = diff.split('\n')
  const hasExplicitFocus = lines.some((line) => line.startsWith('>'))
  // Older compressed drafts kept five context lines before the target without a focus marker.
  const legacyFocusIndex =
    !hasExplicitFocus &&
    /^\.\.\. \[\d+ lines omitted\] \.\.\.$/.test(lines[1] ?? '')
      ? 1 + 1 + LEGACY_LINES_BEFORE
      : null
  let newLine: number | null = null
  return lines.map((text, index) => {
    const focused = text.startsWith('>')
    const displayText = focused ? text.slice(1) : text
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(displayText)
    if (hunk !== null) {
      newLine = Number(hunk[1])
      return { text: displayText, kind: 'marker', focused: false }
    }
    if (displayText.startsWith('+') && !displayText.startsWith('+++')) {
      const lineFocused =
        focused ||
        (!hasExplicitFocus &&
          legacyFocusIndex === null &&
          newLine === focusLine) ||
        legacyFocusIndex === index
      if (newLine !== null) newLine += 1
      return { text: displayText, kind: 'added', focused: lineFocused }
    }
    if (displayText.startsWith('-') && !displayText.startsWith('---')) {
      return {
        text: displayText,
        kind: 'removed',
        focused: focused || legacyFocusIndex === index,
      }
    }
    if (displayText.startsWith(' ') || displayText.length === 0) {
      const lineFocused =
        focused ||
        (!hasExplicitFocus &&
          legacyFocusIndex === null &&
          newLine === focusLine) ||
        legacyFocusIndex === index
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
    .map((block, index) => {
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
        const threadSeparator =
          blocks[index + 1]?.startsWith('<!-- thread: ') === true
            ? '\n\n'
            : '\n'
        updated = prefix + '\n' + changes.reply + threadSeparator
      }
      return updated
    })
    .join('')
}

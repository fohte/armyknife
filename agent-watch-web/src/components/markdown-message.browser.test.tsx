import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { MarkdownMessage } from '#components/markdown-message'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

function getMarkdownLayoutSnapshot() {
  const message = container.firstElementChild
  const paragraph = container.querySelector('p')
  const range = document.createRange()
  if (message === null || paragraph === null) {
    throw new Error('Expected Markdown message blocks to render')
  }
  range.selectNodeContents(paragraph)
  const blocks = Array.from(message.children)
  const blockGaps = blocks.slice(1).map((block, index) => {
    const previous = blocks[index]
    if (previous === undefined) throw new Error('Expected a preceding block')
    return Math.round(
      block.getBoundingClientRect().top -
        previous.getBoundingClientRect().bottom,
    )
  })
  const paragraphLineCount = new Set(
    Array.from(range.getClientRects(), (rect) => Math.round(rect.top)),
  ).size

  return [
    getComputedStyle(message).whiteSpace,
    getComputedStyle(paragraph).whiteSpace,
    paragraphLineCount,
    blocks.map((block) => block.tagName),
    blockGaps,
  ]
}

it('preserves paragraph line breaks without adding blank space between blocks', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <MarkdownMessage>{`## Retry policy

First line
Second line

- Retry transient failures
- Stop after three attempts

| Attempt | Delay |
| --- | --- |
| 1 | 100 ms |

~~~rust
let attempts = 3;
~~~`}</MarkdownMessage>,
    )
  })

  expect(getMarkdownLayoutSnapshot()).toEqual([
    'normal',
    'pre-line',
    2,
    ['H2', 'P', 'UL', 'TABLE', 'PRE'],
    [12, 12, 12, 12],
  ])
})

it('shows distinct markers for nested unordered and ordered lists', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <MarkdownMessage>{`- First item
  - Nested item
    - Third-level item
  1. Nested numbered item
     1. Third-level numbered item

1. First step
   - Nested unordered item
     1. Third-level numbered item`}</MarkdownMessage>,
    )
  })

  const listMarkers = Array.from(
    container.querySelectorAll('ul, ol'),
    (list) => [list.tagName, getComputedStyle(list).listStyleType],
  )

  expect(listMarkers).toEqual([
    ['UL', 'disc'],
    ['UL', 'circle'],
    ['UL', 'square'],
    ['OL', 'lower-alpha'],
    ['OL', 'lower-roman'],
    ['OL', 'decimal'],
    ['UL', 'circle'],
    ['OL', 'lower-roman'],
  ])
})

it('links known session IDs in messages and code while leaving unknown IDs plain', async () => {
  const linkedId = '11111111-1111-4111-8111-111111111111'
  const unknownId = '22222222-2222-4222-8222-222222222222'
  const linkedSession: Session = {
    id: linkedId,
    category: 'in_progress',
    isPaused: false,
    kind: 'Plan',
    title: 'Inspect the retry path',
    preview: null,
    elapsedSeconds: 30,
    unread: false,
    task: null,
    repo: 'example/toolkit',
    project: 'example toolkit',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  }
  const onSelectSession = vi.fn<(id: string) => void>()
  root = createRoot(container)
  act(() => {
    root?.render(
      <MarkdownMessage
        onSelectSession={onSelectSession}
        sessions={[linkedSession]}
      >
        {`The session ${linkedId} is active, while ${unknownId} is unknown.

Use \`${linkedId}\` to open it.

~~~text
${linkedId}
${unknownId}
~~~`}
      </MarkdownMessage>,
    )
  })

  const links = Array.from(container.querySelectorAll('a'))
  const firstLink = links[0]
  if (firstLink === undefined) throw new Error('Expected a session link')
  await userEvent.click(firstLink)

  const getActual = () => ({
    text: container.textContent,
    links: links.map((link) => ({
      text: link.textContent,
      title: link.title,
      href: link.getAttribute('href'),
      insideCode: link.closest('code') !== null,
      insidePre: link.closest('pre') !== null,
    })),
    selectedIds: onSelectSession.mock.calls.map(([id]) => id),
  })
  const expected = {
    text: `The session ${linkedId} is active, while ${unknownId} is unknown.\nUse ${linkedId} to open it.\n${linkedId}\n${unknownId}\n`,
    links: [
      {
        text: linkedId,
        title: 'Inspect the retry path\nexample toolkit\nIn progress',
        href: `?session=${linkedId}`,
        insideCode: false,
        insidePre: false,
      },
      {
        text: linkedId,
        title: 'Inspect the retry path\nexample toolkit\nIn progress',
        href: `?session=${linkedId}`,
        insideCode: true,
        insidePre: false,
      },
      {
        text: linkedId,
        title: 'Inspect the retry path\nexample toolkit\nIn progress',
        href: `?session=${linkedId}`,
        insideCode: true,
        insidePre: true,
      },
    ],
    selectedIds: [linkedId],
  }
  expect(getActual()).toEqual(expected)
})

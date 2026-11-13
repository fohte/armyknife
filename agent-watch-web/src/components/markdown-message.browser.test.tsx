import type { Result } from 'neverthrow'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { MarkdownMessage } from '#components/markdown-message'
import { PeerMessage } from '#components/peer-message'
import type {
  SessionOperation,
  SessionOperationError,
} from '#session-operations'
import { submitSessionOperation } from '#session-operations'
import type { Session } from '#types'

const { runSessionOperationMock } = vi.hoisted(() => ({
  runSessionOperationMock:
    vi.fn<
      (
        operation: SessionOperation,
      ) => Promise<Result<unknown, SessionOperationError>>
    >(),
}))

vi.mock('#session-operation-context', async (importOriginal) => {
  const original =
    await importOriginal<typeof import('#session-operation-context')>()
  return {
    ...original,
    useSessionOperations: () => ({
      ...original.useSessionOperations(),
      run: runSessionOperationMock,
    }),
  }
})

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  runSessionOperationMock.mockReset()
  runSessionOperationMock.mockImplementation(async (operation) => {
    return await submitSessionOperation(operation)
  })
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
  const hasHighlightedCode =
    message.querySelector('.syntax-code-block code span[style]') !== null

  return [
    getComputedStyle(message).whiteSpace,
    getComputedStyle(paragraph).whiteSpace,
    paragraphLineCount,
    blocks.map((block) => block.tagName),
    blockGaps,
    hasHighlightedCode,
  ]
}

function getPeerMessageLayoutSnapshot() {
  const peerContent = container.querySelector('.chat-peer-content')
  const table = container.querySelector('.chat-peer-text table')
  if (
    !(peerContent instanceof HTMLElement) ||
    !(table instanceof HTMLElement)
  ) {
    throw new Error('Expected peer message table content to render')
  }

  return {
    contentOverflows: peerContent.scrollWidth > peerContent.clientWidth,
    tableExceedsContent:
      table.getBoundingClientRect().width >
      peerContent.getBoundingClientRect().width,
  }
}

function getTaggedMarkdownSnapshot() {
  const message = container.firstElementChild
  if (!(message instanceof HTMLElement)) {
    throw new Error('Expected a tagged Markdown message to render')
  }
  const blocks = Array.from(message.children)
  const blockGaps = blocks.slice(1).map((block, index) => {
    const previous = blocks[index]
    if (previous === undefined) throw new Error('Expected a preceding block')
    return Math.round(
      block.getBoundingClientRect().top -
        previous.getBoundingClientRect().bottom,
    )
  })

  return {
    blockGaps,
    blockTags: blocks.map((block) => block.tagName),
    heading: message.querySelector('h2')?.textContent ?? null,
    hasOuterTag: message.querySelector('delegation-update') !== null,
    text: message.textContent,
    whiteSpace: getComputedStyle(message).whiteSpace,
  }
}

it('preserves paragraph line breaks without adding blank space between blocks', async () => {
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
const maxPreviewItems = 7;
~~~`}</MarkdownMessage>,
    )
  })

  await act(async () => {
    const { highlightCode } = await import('./syntax-highlighter')
    await highlightCode('const maxPreviewItems = 7;', 'rust')
  })

  expect(getMarkdownLayoutSnapshot()).toEqual([
    'normal',
    'pre-line',
    2,
    ['H2', 'P', 'UL', 'TABLE', 'PRE'],
    [12, 12, 12, 12],
    true,
  ])
})

it('renders a single outer tag as a label and formatted Markdown body', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <MarkdownMessage>{`<delegation-update>
A summary.

- First item
- Second item
</delegation-update>`}</MarkdownMessage>,
    )
  })

  expect(getTaggedMarkdownSnapshot()).toEqual({
    blockGaps: [12, 12],
    blockTags: ['H2', 'P', 'UL'],
    heading: 'delegation-update',
    hasOuterTag: false,
    text: `delegation-update
A summary.

First item
Second item
`,
    whiteSpace: 'normal',
  })
})

it('keeps wide tables inside peer message content', () => {
  container.style.cssText =
    'box-sizing: border-box; display: flex; flex-direction: column; padding: 24px; width: 640px;'
  const target = {
    id: 'linked-session',
    title: 'Build verification',
    category: 'in_progress' as const,
    isPaused: false,
    task: null,
    repo: 'example/project',
  }
  root = createRoot(container)
  act(() => {
    root?.render(
      <PeerMessage
        message={{
          type: 'peer',
          direction: 'from',
          session_id: target.id,
          content: [
            {
              type: 'text',
              text: `| Check | Result | Details | Owner | Notes |\n| --- | --- | --- | --- | --- |\n| ${'long-entry-'.repeat(12)} | passed | ${'additional-description-'.repeat(12)} | agent | ${'follow-up-'.repeat(12)} |`,
            },
          ],
        }}
        onSelectSession={() => undefined}
        sessions={[target]}
      />,
    )
  })

  expect(getPeerMessageLayoutSnapshot()).toEqual({
    contentOverflows: false,
    tableExceedsContent: false,
  })
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
  const fencedCode = `let sessionId = "${linkedId}";
let unknownId = "${unknownId}";
`
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

~~~rust
${fencedCode}~~~`}
      </MarkdownMessage>,
    )
  })

  await act(async () => {
    const { highlightCode } = await import('./syntax-highlighter')
    await highlightCode(fencedCode, 'rust')
  })

  const paragraphLink = container.querySelector('p a')
  const codeLink = container.querySelector('.syntax-code-block a')
  if (paragraphLink === null || codeLink === null) {
    throw new Error(
      'Expected session links to render in prose and highlighted code',
    )
  }
  await userEvent.click(paragraphLink)
  await userEvent.click(codeLink)

  const getActual = () => ({
    text: container.textContent,
    highlightedCode:
      container.querySelector('.syntax-code-block code span[style]') !== null,
    links: Array.from(container.querySelectorAll('a'), (link) => ({
      text: link.textContent,
      title: link.title,
      underlineStyle: getComputedStyle(link).textDecorationStyle,
      href: link.getAttribute('href'),
      insideCode: link.closest('code') !== null,
      insidePre: link.closest('pre') !== null,
    })),
    selectedIds: onSelectSession.mock.calls.map(([id]) => id),
  })
  const expected = {
    text: `The session ${linkedId} is active, while ${unknownId} is unknown.\nUse ${linkedId} to open it.\nlet sessionId = "${linkedId}";\nlet unknownId = "${unknownId}";\n`,
    highlightedCode: true,
    links: [
      {
        text: linkedId,
        title: '',
        underlineStyle: 'dotted',
        href: `?session=${linkedId}`,
        insideCode: false,
        insidePre: false,
      },
      {
        text: linkedId,
        title: '',
        underlineStyle: 'dotted',
        href: `?session=${linkedId}`,
        insideCode: true,
        insidePre: false,
      },
      {
        text: linkedId,
        title: '',
        underlineStyle: 'dotted',
        href: `?session=${linkedId}`,
        insideCode: true,
        insidePre: true,
      },
    ],
    selectedIds: [linkedId, linkedId],
  }
  expect(getActual()).toEqual(expected)
})

it('confirms the cwd before execution and shows the returned status', async () => {
  const requests: { url: string; body: unknown }[] = []
  const onCommandExecuted = vi.fn()
  const execution: { resolve?: (response: Response) => void } = {}
  vi.stubGlobal(
    'fetch',
    vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.href
            : input.url
      const bodyText = init?.body
      const body = JSON.parse(
        typeof bodyText === 'string' ? bodyText : '{}',
      ) as unknown
      requests.push({ url, body })
      if (url.endsWith('/commands/execute')) {
        return new Promise<Response>((resolve) => {
          execution.resolve = resolve
        })
      }
      return Promise.resolve(
        new Response(JSON.stringify({ cwd: '/workspace/example' }), {
          headers: { 'Content-Type': 'application/json' },
        }),
      )
    }),
  )
  root = createRoot(container)
  act(() => {
    root?.render(
      <MarkdownMessage
        onCommandExecuted={onCommandExecuted}
        sessionId="sample-session"
      >
        {'Run `! printf safe`.'}
      </MarkdownMessage>,
    )
  })

  const runButton = container.querySelector('button')
  if (!(runButton instanceof HTMLButtonElement)) {
    throw new Error('Expected a command button')
  }
  await act(async () => userEvent.click(runButton))
  const dialog = document.body.querySelector('[role="dialog"]')
  if (!(dialog instanceof HTMLElement)) {
    throw new Error('Expected a confirmation dialog')
  }
  const confirmButton = Array.from(dialog.querySelectorAll('button')).find(
    (button) => button.textContent === 'Run command',
  )
  if (!(confirmButton instanceof HTMLButtonElement)) {
    throw new Error('Expected the confirmation button')
  }
  const confirmation = {
    cwd: Array.from(
      dialog.querySelectorAll('dd code'),
      (node) => node.textContent,
    ),
    commands: Array.from(
      dialog.querySelectorAll('dd pre'),
      (node) => node.textContent,
    ),
  }
  await act(async () => userEvent.click(confirmButton))
  const disabledWhileRunning = runButton.disabled
  const completeExecution = execution.resolve
  if (completeExecution === undefined) {
    throw new Error('Expected the execution request to start')
  }
  await act(async () => {
    completeExecution(
      new Response(
        JSON.stringify({ exitCode: 0, succeeded: true, timedOut: false }),
        { headers: { 'Content-Type': 'application/json' } },
      ),
    )
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const getSnapshot = () => ({
    confirmation,
    dialog: document.body.querySelector('[role="dialog"]')?.textContent ?? null,
    status: container.querySelector('[role="status"]')?.textContent ?? null,
    disabledWhileRunning,
    refreshCalls: onCommandExecuted.mock.calls.length,
    requests,
    operations: runSessionOperationMock.mock.calls.map(
      ([operation]) => operation,
    ),
  })
  expect(getSnapshot()).toEqual({
    confirmation: {
      cwd: ['/workspace/example'],
      commands: ['printf safe'],
    },
    dialog: null,
    status: 'Completed',
    disabledWhileRunning: true,
    refreshCalls: 1,
    requests: [
      {
        url: '/api/sessions/sample-session/commands/preview',
        body: {
          messageText: 'Run `! printf safe`.',
          commands: ['printf safe'],
        },
      },
      {
        url: '/api/sessions/sample-session/commands/execute',
        body: {
          messageText: 'Run `! printf safe`.',
          commands: ['printf safe'],
        },
      },
    ],
    operations: [
      {
        type: 'command_preview',
        sessionId: 'sample-session',
        messageText: 'Run `! printf safe`.',
        commands: ['printf safe'],
      },
      {
        type: 'command_execute',
        sessionId: 'sample-session',
        messageText: 'Run `! printf safe`.',
        commands: ['printf safe'],
      },
    ],
  })
})

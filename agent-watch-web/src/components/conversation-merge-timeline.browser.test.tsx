import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import {
  type AsyncAction,
  AsyncActionFeedbackProvider,
} from '#async-action-feedback'
import { ConversationPaneView } from '#components/conversation-pane-view'
import { ConversationTimeline } from '#components/conversation-timeline'
import type { ChatItem, Session, SessionActions } from '#types'

const pullRequest = {
  number: 7042,
  state: 'Open' as const,
  owner: 'example-org',
  repo: 'example-service',
  title: 'Update the example service',
  url: 'https://github.com/example-org/example-service/pull/7042',
  headSha: 'fixture-head-sha',
  ciState: 'Successful' as const,
  isCurrentBranch: true,
}

const session: Session = {
  id: 'merge-timeline-session',
  category: 'in_progress',
  isPaused: false,
  kind: 'Work',
  title: 'Review the sample change',
  preview: null,
  elapsedSeconds: 0,
  unread: false,
  task: null,
  repo: 'example-org/example-service',
  project: 'example-service',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
  pullRequests: [pullRequest],
}

const sessionActions: SessionActions = {
  message: true,
  answerQuestion: true,
  permission: { approve: true, reject: true },
  interrupt: true,
  openInTerminal: true,
  rewind: false,
}

const initialMessages: ChatItem[] = [
  { type: 'assistant', text: 'The change is ready for review.' },
  { type: 'assistant', text: 'I am starting the merge.' },
]

let container: HTMLDivElement
let root: Root | null = null
let resolveMerge: ((response: Response) => void) | undefined

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function TimelineHarness({ initiallyLoading }: { initiallyLoading: boolean }) {
  const [messages, setMessages] = useState<readonly ChatItem[]>(
    initiallyLoading ? [] : initialMessages,
  )
  const [loading, setLoading] = useState(initiallyLoading)
  return (
    <>
      <button
        type="button"
        onClick={() => {
          setMessages((current) =>
            initiallyLoading
              ? [
                  ...initialMessages,
                  {
                    type: 'assistant',
                    text: 'The merge is ready for another check.',
                  },
                ]
              : [
                  ...current,
                  {
                    type: 'assistant',
                    text: 'The merge is ready for another check.',
                  },
                ],
          )
          setLoading(false)
        }}
      >
        {initiallyLoading ? 'Load conversation' : 'Add a later message'}
      </button>
      <ConversationPaneView
        session={session}
        messages={messages}
        start={10}
        actions={sessionActions}
        loading={loading}
        error={null}
        onAction={() => Promise.resolve(true)}
        onSelectSession={() => {}}
      />
    </>
  )
}

function render(initiallyLoading = false) {
  root?.render(
    <AsyncActionFeedbackProvider>
      <TimelineHarness initiallyLoading={initiallyLoading} />
    </AsyncActionFeedbackProvider>,
  )
}

function timelineSnapshot() {
  return Array.from(
    container.querySelectorAll(
      '.conversation-scroll .chat-assistant-message, .conversation-scroll .async-action-item--line',
    ),
    (element) => element.textContent,
  )
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal(
    'fetch',
    vi.fn<typeof fetch>(
      () =>
        new Promise((resolve) => {
          resolveMerge = resolve
        }),
    ),
  )
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  resolveMerge = undefined
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

async function clickButton(button: HTMLButtonElement) {
  await act(async () => userEvent.click(button))
}

async function confirmMerge() {
  await clickButton(
    requireElement(container.querySelector('.merge-band-action')),
  )
  await clickButton(
    requireElement(
      [...document.body.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ),
  )
}

it('keeps a merge result between the messages present when it started and later messages', async () => {
  vi.spyOn(Date, 'now').mockReturnValue(new Date(2034, 2, 19, 12, 3).getTime())
  act(() => {
    render()
  })
  await confirmMerge()
  await clickButton(
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Add a later message',
      ) ?? null,
    ),
  )

  const merging = timelineSnapshot()
  await act(async () => {
    resolveMerge?.(Response.json({ result: 'merged' }))
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (
        timelineSnapshot().some((text) => text === 'Merged PR #7042 · 12:03')
      ) {
        return
      }
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const getSnapshots = () => [merging, timelineSnapshot()]
  const expectedSnapshots = [
    [
      'The change is ready for review.',
      'I am starting the merge.',
      'Merging PR #7042…',
      'The merge is ready for another check.',
    ],
    [
      'The change is ready for review.',
      'I am starting the merge.',
      'Merged PR #7042 · 12:03',
      'The merge is ready for another check.',
    ],
  ]
  expect(getSnapshots()).toEqual(expectedSnapshots)
})

it('does not use the action start time when its completion time is missing', () => {
  const action: AsyncAction = {
    id: 1,
    sessionId: session.id,
    sessionTitle: session.title,
    kind: 'merge',
    status: 'merged',
    pullRequestNumber: pullRequest.number,
    timelineIndex: 0,
    createdAt: new Date(2034, 2, 19, 12, 3).getTime(),
  }
  act(() => {
    root?.render(
      <ConversationTimeline
        actions={[action]}
        hasActiveTool={false}
        hasOlderMessages={false}
        loading={false}
        loadingOlder={false}
        messageCount={0}
        messages={[]}
        onLoadOlder={() => {}}
        onScroll={() => {}}
        scrollRef={null}
        sessionId={session.id}
        sessionTitle={session.title}
        start={0}
      />,
    )
  })

  const snapshot = () => ({
    result: container.querySelector('.async-action-item--line')?.textContent,
    time:
      container.querySelector('.async-action-item-time')?.textContent ?? null,
  })
  expect(snapshot()).toEqual({ result: 'Merged PR #7042', time: null })
})

it('keeps merge feedback visible when the conversation boundary is not loaded yet', async () => {
  act(() => {
    render(true)
  })
  await confirmMerge()
  const loading = timelineSnapshot()
  await clickButton(
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Load conversation',
      ) ?? null,
    ),
  )

  const getSnapshots = () => [loading, timelineSnapshot()]
  const expectedSnapshots = [
    ['Merging PR #7042…'],
    [
      'The change is ready for review.',
      'I am starting the merge.',
      'The merge is ready for another check.',
      'Merging PR #7042…',
    ],
  ]
  expect(getSnapshots()).toEqual(expectedSnapshots)

  await act(async () => {
    resolveMerge?.(Response.json({ result: 'merged' }))
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })
})

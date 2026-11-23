import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import {
  AsyncActionFeedbackProvider,
  useAsyncActionStarter,
} from '#async-action-feedback'
import { ConversationPaneView } from '#components/conversation-pane-view'
import type { ChatItem, Session, SessionActions } from '#types'

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
  repo: 'example/service',
  project: 'service maintenance',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const actions: SessionActions = {
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

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function TimelineHarness({
  messages,
  onMessagesChange,
  run,
}: {
  messages: readonly ChatItem[]
  onMessagesChange: (messages: readonly ChatItem[]) => void
  run: () => Promise<{ status: 'merged' }>
}) {
  const startAsyncAction = useAsyncActionStarter(() => undefined)
  return (
    <>
      <button
        type="button"
        onClick={() => {
          startAsyncAction({
            kind: 'merge',
            sessionId: session.id,
            sessionTitle: session.title,
            pullRequestNumber: 7042,
            timelineIndex: 12,
            run,
          })
        }}
      >
        Start merge
      </button>
      <button
        type="button"
        onClick={() => {
          onMessagesChange([
            ...messages,
            {
              type: 'assistant',
              text: 'The merge is ready for another check.',
            },
          ])
        }}
      >
        Add a later message
      </button>
      <ConversationPaneView
        session={session}
        messages={messages}
        start={10}
        actions={actions}
        loading={false}
        error={null}
        onAction={() => Promise.resolve(true)}
        onSelectSession={() => undefined}
      />
    </>
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
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('keeps a merge result between the messages present when it started and later messages', async () => {
  let resolveMerge: ((result: { status: 'merged' }) => void) | undefined
  const run = () =>
    new Promise<{ status: 'merged' }>((resolve) => {
      resolveMerge = resolve
    })
  let currentMessages: readonly ChatItem[] = initialMessages
  const onMessagesChange = (messages: readonly ChatItem[]) => {
    currentMessages = messages
    root?.render(
      <AsyncActionFeedbackProvider>
        <TimelineHarness
          messages={currentMessages}
          onMessagesChange={onMessagesChange}
          run={run}
        />
      </AsyncActionFeedbackProvider>,
    )
  }
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <TimelineHarness
          messages={currentMessages}
          onMessagesChange={onMessagesChange}
          run={run}
        />
      </AsyncActionFeedbackProvider>,
    )
  })

  await act(async () => {
    await userEvent.click(requireElement(container.querySelector('button')))
  })
  await act(async () => {
    await userEvent.click(
      requireElement(container.querySelectorAll('button').item(1)),
    )
  })
  const merging = timelineSnapshot()
  await act(async () => {
    resolveMerge?.({ status: 'merged' })
    await Promise.resolve()
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
      'Merged PR #7042',
      'The merge is ready for another check.',
    ],
  ]
  expect(getSnapshots()).toEqual(expectedSnapshots)
})

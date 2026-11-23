import type { Decorator, Meta, StoryObj } from '@storybook/react-vite'
import { Fragment, useRef } from 'react'

import type { AsyncAction } from '#async-action-feedback'
import { ConversationMessageRow } from '#components/conversation-message-row'
import { ConversationTimeline } from '#components/conversation-timeline'
import type { ChatItem, SessionActions } from '#types'

const sessionId = 'conversation-timeline-story'
const sessionTitle = 'Review the sample change'

const sessionActions: SessionActions = {
  message: true,
  answerQuestion: true,
  permission: { approve: true, reject: true },
  interrupt: true,
  openInTerminal: true,
  rewind: false,
}

const mergeFailure: AsyncAction = {
  id: 0,
  sessionId,
  sessionTitle,
  kind: 'merge',
  status: 'mergeFailed',
  pullRequestNumber: 7042,
  timelineIndex: 2,
  error: 'The base branch changed after the merge was confirmed.',
  createdAt: Date.parse('2034-03-19T10:20:00.000Z'),
}

const failedMessages: ChatItem[] = [
  { type: 'assistant' as const, text: 'The change is ready for review.' },
  { type: 'assistant' as const, text: 'I am starting the merge.' },
  {
    type: 'assistant' as const,
    text: 'Could you check the retry setting as well?',
  },
  {
    type: 'assistant' as const,
    text: 'The retry setting is ready for review.',
  },
]

const mergedActions: AsyncAction[] = [
  {
    id: 1,
    sessionId,
    sessionTitle,
    kind: 'merge',
    status: 'merged',
    pullRequestNumber: 7042,
    timelineIndex: 0,
    createdAt: Date.parse('2034-03-19T12:03:00.000Z'),
    completedAt: new Date(2034, 2, 19, 12, 3).getTime(),
  },
  {
    id: 2,
    sessionId,
    sessionTitle,
    kind: 'merge',
    status: 'merged',
    pullRequestNumber: 7043,
    timelineIndex: 2,
    createdAt: Date.parse('2034-03-19T12:31:00.000Z'),
    completedAt: new Date(2034, 2, 19, 12, 31).getTime(),
  },
]

const mergedMessages: ChatItem[] = [
  {
    type: 'user',
    text: 'Please make the retry limit configurable as well.',
    timestamp: new Date(2034, 2, 19, 12, 5).toISOString(),
  },
  {
    type: 'assistant',
    text: 'I made retry.limit configurable and opened a pull request.',
  },
  {
    type: 'user',
    text: 'Thanks. Check the failure log too.',
    timestamp: new Date(2034, 2, 19, 12, 34).toISOString(),
  },
  {
    type: 'assistant',
    text: 'I am checking the failure log now.',
  },
]

function ConversationTimelineStory({
  actions,
  isMobile,
  messages,
}: {
  actions: readonly AsyncAction[]
  isMobile: boolean
  messages: readonly ChatItem[]
}) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const messageRows = messages.map((message, index) => ({
    index,
    element: (
      <Fragment key={`message-${String(index)}`}>
        <ConversationMessageRow
          actions={sessionActions}
          actionsReady
          isMobile={isMobile}
          selected={false}
          decisionPosition={undefined}
          onSelectDecision={() => undefined}
          message={message}
          pendingAction={null}
          openInTerminalStatus={null}
          onAction={() => Promise.resolve(true)}
          onOpenInTerminal={() => undefined}
          onSelectSession={() => undefined}
          sessionEngine="claude"
          sessionId={sessionId}
          sessions={[]}
          now={new Date('2034-03-19T10:20:00.000Z')}
        />
      </Fragment>
    ),
  }))

  return (
    <div
      className="conversation-pane"
      data-mobile={isMobile ? 'true' : undefined}
    >
      <ConversationTimeline
        actions={actions}
        hasActiveTool={false}
        hasOlderMessages={false}
        loading={false}
        loadingOlder={false}
        messageCount={messageRows.length}
        messages={messageRows}
        onLoadOlder={() => undefined}
        onRetry={() => undefined}
        onScroll={() => undefined}
        scrollRef={scrollRef}
        sessionId={sessionId}
        sessionTitle={sessionTitle}
        start={0}
      />
    </div>
  )
}

const meta = {
  title: 'Conversation/Timeline',
  component: ConversationTimelineStory,
  args: {
    actions: [mergeFailure],
    isMobile: false,
    messages: failedMessages,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '100%', height: '900px', display: 'flex' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ConversationTimelineStory>

export default meta
type Story = StoryObj<typeof meta>

const iPhoneViewportDecorator: Decorator = (Story) => (
  <div
    className="story-mobile-layout"
    data-layout="mobile"
    style={{ width: 'min(390px, 100%)', height: '844px', display: 'flex' }}
  >
    <Story />
  </div>
)

export const DesktopConversationShowsTheFailedMergeAtItsStartPosition: Story = {
  name: 'The failed merge stays before messages that arrive later',
}

export const IPhoneConversationShowsTheFailedMergeAtItsStartPosition: Story = {
  name: 'The iPhone conversation keeps later messages below the failed merge',
  args: { isMobile: true },
  decorators: [iPhoneViewportDecorator],
}

export const DesktopConversationShowsMergedResultsAtTheirStartPositions: Story =
  {
    name: 'Merged results stay between the messages that surround them',
    args: { actions: mergedActions, messages: mergedMessages },
  }

export const IPhoneConversationShowsMergedResultsAtTheirStartPositions: Story =
  {
    name: 'The iPhone conversation keeps later messages below merged results',
    args: { actions: mergedActions, isMobile: true, messages: mergedMessages },
    decorators: [iPhoneViewportDecorator],
  }

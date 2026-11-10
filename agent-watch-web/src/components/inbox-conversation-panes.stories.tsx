import type { Meta, StoryObj } from '@storybook/react-vite'
import { http, HttpResponse } from 'msw'

import { InboxConversationPanes } from '#components/inbox-conversation-panes'
import type { Session } from '#types'

const session: Session = {
  id: 'sample-session',
  isPaused: false,
  category: 'needs_action',
  kind: 'Review',
  title: 'Review the retry policy',
  preview: 'The retry behavior is ready for review.',
  elapsedSeconds: 30,
  unread: true,
  task: null,
  repo: 'example/service',
  project: 'service reliability',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const inboxProps = {
  sessions: [session],
  selectedId: session.id,
  loaded: true,
  error: null,
  showAllInProgress: false,
  onShowMore: () => undefined,
  onSelect: () => undefined,
  laterExpanded: false,
  onToggleLater: () => undefined,
  onLater: () => undefined,
  onToggleToday: () => undefined,
  todaySessions: [],
  onSelectToday: () => undefined,
  inProgressExpanded: false,
  onToggleInProgress: () => undefined,
  doneExpanded: false,
  onToggleDone: () => undefined,
}

const meta = {
  component: InboxConversationPanes,
  args: {
    inboxProps,
    selectedSession: session,
    refreshKey: 0,
    isMobile: false,
    showMobileConversation: false,
    onBack: () => undefined,
    onLater: () => undefined,
    onSelectSession: () => undefined,
  },
  decorators: [
    (Story) => (
      <div
        style={{
          width: '100%',
          maxWidth: '100vw',
          height: '800px',
          display: 'flex',
        }}
      >
        <Story />
      </div>
    ),
  ],
  parameters: {
    msw: {
      handlers: [
        http.get('/api/sessions/:sessionId/messages', () =>
          HttpResponse.json([
            {
              type: 'assistant',
              text: 'The retry policy is ready for review.',
            },
          ]),
        ),
        http.get('/api/sessions/:sessionId/actions', () =>
          HttpResponse.json({
            message: false,
            answerQuestion: false,
            interrupt: false,
            openInTerminal: true,
          }),
        ),
      ],
    },
  },
} satisfies Meta<typeof InboxConversationPanes>

export default meta
type Story = StoryObj<typeof meta>

export const DesktopShowsTheInboxAndConversation: Story = {
  name: 'The desktop shows the inbox beside the selected conversation',
}

export const IPhoneShowsTheInboxBeforeSelection: Story = {
  name: 'The iPhone shows the inbox before a conversation is selected',
  args: {
    inboxProps: { ...inboxProps, selectedId: null },
    selectedSession: null,
    isMobile: true,
  },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        data-layout="mobile"
        style={{ width: '390px', height: '844px', display: 'flex' }}
      >
        <Story />
      </div>
    ),
  ],
}

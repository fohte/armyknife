import type { Meta, StoryObj } from '@storybook/react-vite'

import { ConversationPaneHeader } from '#components/conversation-pane-header'
import type { Session, SessionActions } from '#types'

const session: Session = {
  id: 'sample-session',
  engine: 'claude',
  category: 'needs_action',
  isPaused: false,
  kind: 'Message',
  title: 'Review the request retry flow',
  preview: null,
  elapsedSeconds: 18,
  unread: false,
  task: null,
  repo: 'example/service',
  project: 'service',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const actions: SessionActions = {
  message: true,
  answerQuestion: false,
  permission: { approve: false, reject: false },
  interrupt: false,
  openInTerminal: true,
  rewind: false,
}

const meta = {
  component: ConversationPaneHeader,
  args: {
    session,
    sessions: [],
    actions,
    isMobile: false,
    statusPanelOpen: false,
    backgroundTasks: [],
    openInTerminalStatus: null,
    isToday: false,
    onAction: () => Promise.resolve(true),
    onOpenInTerminal: () => undefined,
    onSelectSession: () => undefined,
    onToggleStatusPanel: () => undefined,
    onToggleToday: () => undefined,
  },
  decorators: [
    (Story) => (
      <div className="conversation-pane" style={{ height: 96 }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ConversationPaneHeader>

export default meta
type Story = StoryObj<typeof meta>

export const SessionContextAndActionsAreVisible: Story = {
  name: 'The header shows session context and actions',
}

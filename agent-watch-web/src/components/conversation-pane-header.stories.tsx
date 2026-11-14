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

export const EditingTheSessionTitle: Story = {
  name: 'The header edits the session title',
  args: {
    titleEditing: true,
  },
}

export const EditingTheSessionTitleOnMobile: Story = {
  name: 'The mobile header edits the session title',
  args: {
    isMobile: true,
    titleEditing: true,
  },
}

export const SessionPullRequestCanBeMerged: Story = {
  name: 'The header offers to merge the session pull request',
  args: {
    session: {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example',
        repo: 'service',
        url: 'https://github.com/example/service/pull/42',
        headSha: 'sample-head-sha',
      },
    },
  },
}

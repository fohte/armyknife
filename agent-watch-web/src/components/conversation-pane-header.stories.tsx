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

export const MobileHeaderKeepsManySessionDetailsAndShowsActionsMenu: Story = {
  name: 'The mobile header keeps a busy session compact with its actions menu open',
  args: {
    isMobile: true,
    mobileActionsMenuOpen: true,
    isToday: true,
    session: {
      ...session,
      category: 'done',
      title:
        'Review the retry strategy and keep the delivery schedule consistent across services',
      repo: 'example/long-running-service-name',
      task: {
        id: 'sample-task-primary',
        number: 2048,
        title: 'Improve request reliability',
        priority: 1,
      },
      tasks: [
        {
          id: 'sample-task-primary',
          number: 2048,
          title: 'Improve request reliability',
          priority: 1,
        },
        {
          id: 'sample-task-secondary',
          number: 2049,
          title: 'Document retry limits',
          priority: 2,
        },
        {
          id: 'sample-task-tertiary',
          number: 2050,
          title: 'Add delivery metrics',
          priority: 3,
        },
      ],
    },
    backgroundTasks: [
      {
        id: 'sample-background-task-a',
        description: 'Review the request flow',
        elapsedSeconds: 24,
        kind: 'subagent',
      },
      {
        id: 'sample-background-task-b',
        description: 'Check delivery tests',
        elapsedSeconds: 12,
        kind: 'background_task',
      },
    ],
    onLater: () => undefined,
    onClose: () => undefined,
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

export const SessionHasMultiplePullRequests: Story = {
  name: 'The header shows every pull request submitted by the session',
  args: {
    session: {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-repo',
        url: 'https://github.com/example-org/example-repo/pull/42',
        headSha: 'sample-head-sha',
      },
      pullRequests: [
        {
          number: 42,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-repo',
          url: 'https://github.com/example-org/example-repo/pull/42',
          headSha: 'sample-head-sha',
        },
        {
          number: 43,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-repo',
          url: 'https://github.com/example-org/example-repo/pull/43',
          headSha: 'other-head-sha',
        },
      ],
    },
  },
}

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
    (Story, context) => (
      <div
        className="conversation-pane"
        data-mobile={context.args.isMobile}
        style={{
          height: context.args.isMobile
            ? (context.args.session.pullRequests?.length ?? 0) > 0 ||
              context.args.session.pullRequest !== null
              ? 130
              : 88
            : 96,
        }}
      >
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
    session: { ...session, title: 'Edit' },
  },
}

export const MobileHeaderShowsTaskAndRepository: Story = {
  name: 'The mobile header keeps the task and repository on its second row',
  args: {
    isMobile: true,
    session: {
      ...session,
      category: 'done',
      title: 'Retry',
      repo: 'example/service',
      task: {
        id: 'sample-task-primary',
        number: 8,
        title: 'Fix',
        priority: null,
      },
      tasks: [
        {
          id: 'sample-task-primary',
          number: 8,
          title: 'Fix',
          priority: null,
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
  name: 'The header counts the other pull requests from the session',
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

export const MobileHeaderShowsPullRequestSheet: Story = {
  name: 'The mobile header shows the open pull request sheet below the title',
  args: {
    isMobile: true,
    pullRequestsOpen: true,
    session: {
      ...session,
      title: 'Show delivery status in the session header',
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-repo',
        title: 'Show delivery status in the session header',
        url: 'https://github.com/example-org/example-repo/pull/42',
        ciState: 'InProgress',
        isCurrentBranch: true,
        headSha: 'fixture-head-sha',
      },
      pullRequests: [
        {
          number: 42,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-repo',
          title: 'Show delivery status in the session header',
          url: 'https://github.com/example-org/example-repo/pull/42',
          ciState: 'InProgress',
          isCurrentBranch: true,
          headSha: 'fixture-head-sha',
        },
        {
          number: 43,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-infra',
          title: 'Add delivery retry settings',
          url: 'https://github.com/example-org/example-infra/pull/43',
          ciState: 'Successful',
          isCurrentBranch: false,
          headSha: 'fixture-other-sha',
        },
      ],
    },
  },
}

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
  parameters: { layout: 'fullscreen' },
  args: {
    session,
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
    onClose: () => undefined,
  },
  decorators: [
    (Story, context) => (
      <div
        className="conversation-pane"
        data-mobile={context.args.isMobile}
        style={{
          width: context.args.isMobile ? 390 : undefined,
          height: context.args.isMobile ? 88 : 96,
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
  name: 'The desktop header shows its pull request button',
  args: {
    session: {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example',
        repo: 'service',
        url: 'https://github.com/example/service/pull/42',
        ciState: 'Successful',
        isCurrentBranch: true,
        headSha: 'sample-head-sha',
      },
    },
  },
}

export const SessionPullRequestWaitsForChecks: Story = {
  name: 'The header shows that the pull request checks are still running',
  args: {
    session: {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example',
        repo: 'service',
        ciState: 'InProgress',
        isCurrentBranch: true,
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
  name: 'The desktop pull request button opens the pull request list',
  args: {
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

export const PhoneTitleFitsOnOneLine: Story = {
  name: 'The phone header shows a short title on one line',
  args: {
    isMobile: true,
    session: { ...session, title: 'Short title' },
  },
}

export const PhoneTitleTruncatesAfterTwoLines: Story = {
  name: 'The phone header truncates a long title after two lines',
  args: {
    isMobile: true,
    session: {
      ...session,
      title:
        'Review the retry flow across every background task and delivery path before the next release',
    },
  },
}

export const PhoneHeaderShowsNoPullRequests: Story = {
  name: 'The phone header has no pull request status badge',
  args: { isMobile: true, session },
}

export const PhoneHeaderShowsPullRequestStatus: Story = {
  name: 'The phone header shows pull request count and CI status',
  args: {
    isMobile: true,
    session: {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-service',
        ciState: 'InProgress',
        isCurrentBranch: true,
        headSha: 'sample-head-sha',
      },
      pullRequests: [
        {
          number: 42,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-service',
          ciState: 'InProgress',
          isCurrentBranch: true,
          headSha: 'sample-head-sha',
        },
        {
          number: 43,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-infra',
          ciState: 'Successful',
          isCurrentBranch: false,
          headSha: 'sample-other-sha',
        },
      ],
    },
  },
}

export const PhoneNavigationShowsActivityFailure: Story = {
  name: 'The phone navigation button marks failed Activity actions',
  args: { isMobile: true, activityFailed: true },
}

export const PhoneHeaderShowsTheSelectedDesignState: Story = {
  name: 'The phone header shows a long title, failed Activity, and three pull requests',
  args: {
    isMobile: true,
    isToday: true,
    activityFailed: true,
    session: {
      ...session,
      vibeMode: { mode: 'enabled' },
      title: 'Review the delivery retry policy across all services',
      task: {
        id: 'sample-task-phone-header',
        number: 73,
        title: 'Review retry behavior',
        priority: null,
      },
      tasks: [
        {
          id: 'sample-task-phone-header',
          number: 73,
          title: 'Review retry behavior',
          priority: null,
        },
      ],
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-service',
        title: 'Review retry behavior',
        ciState: 'InProgress',
        isCurrentBranch: true,
        headSha: 'sample-head-sha',
      },
      pullRequests: [
        {
          number: 42,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-service',
          title: 'Review retry behavior',
          ciState: 'InProgress',
          isCurrentBranch: true,
          headSha: 'sample-head-sha',
        },
        {
          number: 43,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-service',
          title: 'Update delivery timeouts',
          ciState: 'Successful',
          isCurrentBranch: false,
          headSha: 'sample-other-head-sha',
        },
        {
          number: 44,
          state: 'Open',
          owner: 'example-team',
          repo: 'sample-infra',
          title: 'Add retry metrics',
          ciState: 'Successful',
          isCurrentBranch: false,
          headSha: 'sample-infra-head-sha',
        },
      ],
    },
  },
}

import type { Meta, StoryObj } from '@storybook/react-vite'

import { CommandPalette } from '#components/command-palette'
import type { Session } from '#types'

const sessions: Session[] = [
  {
    id: 'session-review',
    category: 'needs_action',
    isPaused: false,
    kind: 'Question',
    title: 'Review the notification rollout',
    preview: null,
    elapsedSeconds: 3_600,
    unread: true,
    task: {
      id: 'task-notifications',
      number: 42,
      title: 'Improve notifications',
      priority: 1,
    },
    tasks: [
      {
        id: 'task-notifications',
        number: 42,
        title: 'Improve notifications',
        priority: 1,
      },
      {
        id: 'task-delivery',
        number: 43,
        title: 'Document delivery behavior',
        priority: null,
      },
    ],
    repo: 'example/web-app',
    branch: 'feature/notification-rollout',
    canOpenInTerminal: true,
    project: 'notification refresh',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'session-tests',
    category: 'in_progress',
    isPaused: false,
    kind: null,
    title: 'Add delivery retry coverage',
    preview: null,
    elapsedSeconds: 7_200,
    unread: false,
    task: {
      id: 'task-notifications',
      number: 42,
      title: 'Improve notifications',
      priority: 1,
    },
    repo: 'example/web-app',
    branch: 'feature/retry-coverage',
    project: 'notification refresh',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'session-health',
    category: 'needs_action',
    isPaused: false,
    kind: 'Error',
    title: 'Investigate notification delivery errors',
    preview: null,
    elapsedSeconds: 720,
    unread: true,
    task: {
      id: 'task-notifications',
      number: 42,
      title: 'Improve notifications',
      priority: 1,
    },
    repo: 'example/web-app',
    branch: 'feature/notification-health',
    canOpenInTerminal: true,
    project: 'notification refresh',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'session-deploy',
    category: 'needs_action',
    isPaused: false,
    kind: 'Plan',
    title: 'Plan notification deployment',
    preview: null,
    elapsedSeconds: 1800,
    unread: true,
    task: {
      id: 'task-deployment',
      number: 57,
      title: 'Prepare a deployment plan',
      priority: null,
    },
    repo: 'example/web-app',
    branch: 'feature/notification-deploy',
    canOpenInTerminal: true,
    project: 'notification refresh',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'session-docs',
    category: 'done',
    isPaused: false,
    kind: null,
    title: 'Update the setup guide',
    preview: null,
    elapsedSeconds: 21_600,
    unread: false,
    task: null,
    repo: 'example/handbook',
    branch: null,
    project: 'developer guides',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
]

const meta = {
  component: CommandPalette,
  args: {
    sessions,
    recentSessionIds: ['session-tests', 'session-docs'],
    onClose: () => undefined,
    onOpenSession: () => undefined,
    onLater: () => undefined,
    onOpenInTerminal: () => undefined,
    onOpenProject: () => undefined,
    onRunCommand: () => undefined,
  },
} satisfies Meta<typeof CommandPalette>

export default meta
type Story = StoryObj<typeof meta>

export const NothingIsTyped: Story = {
  name: 'The top actions, recent activity, and commands appear by default',
}

export const SearchShowsMatchingDestinations: Story = {
  name: 'Matching sessions, tasks, and projects appear while searching',
  args: { initialQuery: 'notification' },
}

export const TaskScopeShowsMatchingTasks: Story = {
  name: 'The task scope shows matching tq tasks',
  args: { initialQuery: '#notification' },
}

export const BranchSearchShowsSessionActions: Story = {
  name: 'A branch search shows its session action menu',
  args: {
    initialQuery: 'feature/notification-rollout',
    initialActionItemId: 'session-review',
  },
}

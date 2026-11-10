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
    repo: 'example/web-app',
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
    recentSessionIds: ['session-tests'],
    inboxFilterQuery: '',
    onClose: () => undefined,
    onOpenSession: () => undefined,
    onOpenProject: () => undefined,
    onRunCommand: () => undefined,
  },
} satisfies Meta<typeof CommandPalette>

export default meta
type Story = StoryObj<typeof meta>

export const NothingIsTyped: Story = {
  name: 'The top action, recent session, and commands appear by default',
}

export const SearchShowsMatchingDestinations: Story = {
  name: 'Matching sessions, tasks, projects, and commands appear while searching',
  args: { initialQuery: 'notification' },
}

export const TaskScopeShowsMatchingTasks: Story = {
  name: 'The task scope shows matching tq tasks',
  args: { initialQuery: '#notification' },
}

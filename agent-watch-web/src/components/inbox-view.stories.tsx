import type { Meta, StoryObj } from '@storybook/react-vite'

import { InboxView } from '#components/inbox-view'
import type { Session } from '#types'

const sessions: Session[] = [
  {
    id: 'sample-question',
    category: 'needs_action',
    kind: 'Question',
    title: 'Update retry handling',
    preview: 'Should retries stop after three attempts?',
    elapsedSeconds: 8,
    unread: true,
    task: {
      id: 'sample-task',
      number: 2048,
      title: 'Improve request reliability',
      priority: 1,
    },
    repo: 'example/service',
    project: 'sample project',
    parent: null,
    pullRequest: null,
  },
  {
    id: 'sample-work',
    category: 'in_progress',
    kind: null,
    title: 'Add request metrics',
    preview: null,
    elapsedSeconds: 30,
    unread: false,
    task: null,
    repo: 'example/observability',
    project: 'sample project',
    parent: null,
    pullRequest: null,
  },
  {
    id: 'sample-work-secondary',
    category: 'in_progress',
    kind: null,
    title: 'Trace request latency',
    preview: null,
    elapsedSeconds: 90,
    unread: false,
    task: null,
    repo: 'example/observability',
    project: 'sample project',
    parent: null,
    pullRequest: null,
  },
  {
    id: 'sample-work-hidden',
    category: 'in_progress',
    kind: null,
    title: 'Measure cache behavior',
    preview: null,
    elapsedSeconds: 120,
    unread: false,
    task: null,
    repo: 'example/observability',
    project: 'sample project',
    parent: null,
    pullRequest: null,
  },
  {
    id: 'sample-done',
    category: 'done',
    kind: null,
    title: 'Document request limits',
    preview: null,
    elapsedSeconds: 3600,
    unread: false,
    task: null,
    repo: 'example/service',
    project: 'sample project',
    parent: null,
    pullRequest: { number: 2049, state: 'Merged' },
  },
]

const meta = {
  component: InboxView,
  args: {
    sessions,
    selectedId: 'sample-question',
    loaded: true,
    error: null,
    showAllInProgress: false,
    onShowMore: () => undefined,
    onSelect: () => undefined,
  },
  decorators: [
    (Story) => (
      <div className="story-inbox-container">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof InboxView>

export default meta
type Story = StoryObj<typeof meta>

export const InboxHasSessionsAcrossStatuses: Story = {
  name: 'The inbox groups sessions by action, progress, and completion',
}

export const InboxIsLoading: Story = {
  name: 'The inbox shows a loading state before the first response',
  args: { sessions: [], loaded: false },
}

export const InboxShowsAllActiveSessions: Story = {
  name: 'The inbox reveals every active session when expanded',
  args: { showAllInProgress: true },
}

export const InboxHasNoSessions: Story = {
  name: 'The inbox shows an empty state when there are no sessions',
  args: { sessions: [], loaded: true, selectedId: null },
}

export const InboxCannotLoadSessions: Story = {
  name: 'The inbox explains when a session request fails',
  args: {
    sessions: [],
    loaded: false,
    error: 'Unable to load sessions. Reconnecting…',
  },
}

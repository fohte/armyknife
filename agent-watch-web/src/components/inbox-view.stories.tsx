import type { Meta, StoryObj } from '@storybook/react-vite'

import { InboxView } from '#components/inbox-view'
import type { Session } from '#types'

const sessions: Session[] = [
  {
    id: 'sample-later',
    category: 'later',
    isPaused: false,
    categoryWhenNotLater: 'needs_action',
    kind: 'Question',
    title: 'Review the service retry policy',
    preview: 'Can retries also use backoff?',
    elapsedSeconds: 120,
    unread: true,
    task: null,
    repo: 'example/service',
    project: 'sample project',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
    laterUntil: '2030-01-02T09:00:00.000Z',
  },
  {
    id: 'sample-question',
    category: 'needs_action',
    isPaused: false,
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
    ancestorSessionIds: [],
    parent: null,
    pullRequest: null,
  },
  {
    id: 'sample-work',
    category: 'in_progress',
    isPaused: false,
    kind: null,
    title: 'Add request metrics',
    preview: null,
    elapsedSeconds: 30,
    unread: false,
    task: null,
    repo: 'example/observability',
    project: 'sample project',
    ancestorSessionIds: [],
    parent: null,
    pullRequest: null,
  },
  {
    id: 'sample-work-secondary',
    category: 'in_progress',
    isPaused: false,
    kind: null,
    title: 'Trace request latency',
    preview: null,
    elapsedSeconds: 90,
    unread: false,
    task: null,
    repo: 'example/observability',
    project: 'sample project',
    ancestorSessionIds: [],
    parent: null,
    pullRequest: null,
  },
  {
    id: 'sample-work-hidden',
    category: 'in_progress',
    isPaused: false,
    kind: null,
    title: 'Measure cache behavior',
    preview: null,
    elapsedSeconds: 120,
    unread: false,
    task: null,
    repo: 'example/observability',
    project: 'sample project',
    ancestorSessionIds: [],
    parent: null,
    pullRequest: null,
  },
  {
    id: 'sample-done',
    category: 'done',
    isPaused: false,
    kind: null,
    title: 'Document request limits',
    preview: null,
    elapsedSeconds: 3600,
    unread: false,
    task: null,
    repo: 'example/service',
    project: 'sample project',
    ancestorSessionIds: [],
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
    laterExpanded: true,
    onToggleLater: () => undefined,
    onLater: () => undefined,
    onToggleToday: () => undefined,
    todaySessions: sessions.filter(
      (session) => session.id === 'sample-question',
    ),
    onSelectToday: () => undefined,
    isMobile: false,
    inProgressExpanded: false,
    onToggleInProgress: () => undefined,
    doneExpanded: false,
    onToggleDone: () => undefined,
    onSaveView: () => undefined,
    onSaveViewOpenChange: () => undefined,
    onSaveViewNameChange: () => undefined,
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

export const InboxHasNoFilters: Story = {
  name: 'The inbox shows the filter shortcut before any filters are set',
  args: { filterQuery: '' },
}

export const InboxFiltersSessionsByProjectAndKind: Story = {
  name: 'The inbox shows project and kind filters as removable chips',
  args: { filterQuery: 'project:sample project kind:question,review' },
}

export const InboxFiltersCanBeSavedAsAView: Story = {
  name: 'The filtered inbox offers a button to save its view',
  args: { filterQuery: 'project:sample project kind:question,review' },
}

export const InboxNamesASavedView: Story = {
  name: 'The filtered inbox shows the saved view name field',
  args: {
    filterQuery: 'project:sample project kind:question,review',
    saveViewOpen: true,
    saveViewName: 'Sample questions',
  },
}

export const InboxFilterDialogIsOpen: Story = {
  name: 'The filter dialog shows available session values and counts',
  args: { filterQuery: 'project:sample project', filterDialogOpen: true },
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

export const InboxFitsAnIPhone: Story = {
  name: 'The inbox keeps its actions usable on an iPhone',
  args: {
    isMobile: true,
    laterExpanded: false,
    todaySessions: sessions.filter(
      (session) => session.id === 'sample-question',
    ),
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

export const IPhoneInboxHasNoLaterSessions: Story = {
  name: 'The iPhone inbox shows its empty Later section',
  args: {
    isMobile: true,
    sessions: [],
    selectedId: null,
    laterExpanded: false,
    todaySessions: [],
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

export const IPhoneInboxShowsExpandedProgress: Story = {
  name: 'The iPhone inbox shows expanded active sessions',
  args: {
    isMobile: true,
    laterExpanded: false,
    inProgressExpanded: true,
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

export const IPhoneInboxShowsCompletedSessions: Story = {
  name: 'The iPhone inbox shows expanded completed sessions',
  args: {
    isMobile: true,
    laterExpanded: false,
    doneExpanded: true,
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

import type { Meta, StoryObj } from '@storybook/react-vite'

import { AgentWatchNav } from '#components/agent-watch-nav'
import type { Session } from '#types'

const todaySession: Session = {
  id: 'today-session',
  engine: 'claude',
  workType: 'PM',
  category: 'needs_action',
  isPaused: false,
  kind: 'Question',
  title: 'Check the retry policy',
  preview: 'Choose a limit.',
  elapsedSeconds: 10,
  unread: true,
  task: {
    id: 'sample-task',
    number: 17,
    title: 'Improve request reliability',
    priority: 2,
  },
  repo: 'example/service',
  project: 'sample project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
  isToday: true,
}

const todaySessionInProgress: Session = {
  ...todaySession,
  id: 'today-session-in-progress',
  category: 'in_progress',
  kind: null,
  title: 'Review the release notes',
  task: {
    id: 'sample-task',
    number: 17,
    title: 'Review the release notes',
    priority: null,
  },
}

const todaySessionStopped: Session = {
  ...todaySessionInProgress,
  id: 'today-session-paused',
  isPaused: true,
  kind: 'Stopped',
  title: 'Prepare the next deployment window',
}

const meta = {
  component: AgentWatchNav,
  args: {
    activeView: 'inbox',
    paletteShortcut: {
      modifier: 'metaKey',
      label: '⌘K',
      ariaLabel: 'Command K',
    },
    needsActionCount: 3,
    savedViews: [],
    selectedSavedViewId: null,
    workTypeNavGroups: [
      {
        label: 'PM',
        sessions: [
          todaySession,
          {
            ...todaySessionInProgress,
            id: 'pm-working-session',
            workType: 'PM',
          },
        ],
      },
      {
        label: 'Reviews',
        sessions: [],
      },
    ],
    onViewChange: () => undefined,
    onSelectSavedView: () => undefined,
    onDeleteSavedView: () => undefined,
    todaySessions: [todaySession],
    onSelectToday: () => undefined,
    onSelectSession: () => undefined,
    onOpenPalette: () => undefined,
    onNewSession: () => undefined,
    inboxCollapsed: false,
    onToggleInbox: () => undefined,
  },
} satisfies Meta<typeof AgentWatchNav>

export default meta
type Story = StoryObj<typeof meta>

export const InboxIsSelected: Story = {
  name: 'Inbox is selected in the main navigation',
}

export const BoardIsSelected: Story = {
  name: 'Board is selected in the main navigation',
  args: { activeView: 'board' },
}

export const InProgressTodaySession: Story = {
  name: 'An in-progress starred session shows the Working status',
  args: { todaySessions: [todaySessionInProgress] },
}

export const TodaySessionStatusesUseTheirCategoryColors: Story = {
  name: 'Starred sessions show kind icons in category colors',
  args: {
    todaySessions: [
      todaySession,
      todaySessionInProgress,
      { ...todaySession, id: 'today-session-error', kind: 'Error' },
      todaySessionStopped,
    ],
  },
}

export const RecentSessionsAppearNewestFirst: Story = {
  name: 'Recent sessions appear in order of the latest human prompt',
  args: {
    sessions: [
      {
        ...todaySession,
        id: 'recent-session-one',
        title: 'Review the retry policy',
        repo: 'example/service',
        lastPromptedAt: '2026-01-01T10:00:00.000Z',
      },
      {
        ...todaySession,
        id: 'recent-session-two',
        title: 'Update the delivery monitor',
        repo: 'example/observability',
        lastPromptedAt: '2026-01-01T11:00:00.000Z',
      },
    ],
  },
}

export const RecentSessionsAppearOnMobile: Story = {
  name: 'Mobile navigation shows the Recent section',
  args: {
    isMobile: true,
    sessions: [
      {
        ...todaySession,
        lastPromptedAt: '2026-01-01T11:00:00.000Z',
      },
    ],
  },
}

export const MobileNavigationShowsFailedActivity: Story = {
  name: 'The mobile navigation marks Inbox while Activity has a failure',
  args: {
    isMobile: true,
    activityCount: 2,
    activityFailed: true,
  },
  decorators: [
    (StoryComponent) => (
      <div
        style={{
          width: '320px',
          height: '844px',
          background: 'var(--aw-bg-app)',
        }}
      >
        <StoryComponent />
      </div>
    ),
  ],
}

export const InboxIsCollapsed: Story = {
  name: 'The Inbox toggle is highlighted while the Inbox is collapsed',
  args: { inboxCollapsed: true },
}

export const NoWorkTypeIsConfigured: Story = {
  name: 'No work type list appears when no display labels are configured',
  args: { workTypeNavGroups: [] },
}

export const NoSavedViews: Story = {
  name: 'The navigation hides the views section when no views are saved',
}

export const SavedViewsShowCountsAndStatusColors: Story = {
  name: 'Saved views show matching session counts in their status colors',
  args: {
    savedViews: [
      {
        id: 'reviews',
        name: 'Sample reviews',
        query: 'project:sample-project kind:review',
        count: 3,
        countTone: 'needs-action',
      },
      {
        id: 'errors',
        name: 'Errors',
        query: 'kind:error',
        count: 1,
        countTone: 'error',
      },
    ],
  },
}

export const NavigationMatchesTheScreenDesign: Story = {
  name: 'The navigation places views, recent sessions, and PM below the brand row',
  args: {
    savedViews: [
      {
        id: 'reviews',
        name: 'Example reviews',
        query: 'kind:review',
        count: 2,
        countTone: 'needs-action',
      },
    ],
    sessions: [
      {
        ...todaySessionInProgress,
        id: 'recent-session',
        title: 'Review a sample change',
        lastPromptedAt: '2026-01-01T11:00:00.000Z',
      },
    ],
  },
  decorators: [
    (StoryComponent) => (
      <div style={{ width: '240px', height: '900px' }}>
        <StoryComponent />
      </div>
    ),
  ],
}

export const AViewIsSelected: Story = {
  name: 'The selected saved view is highlighted in the navigation',
  args: {
    savedViews: [
      {
        id: 'reviews',
        name: 'Sample reviews',
        query: 'project:sample-project kind:review',
        count: 3,
        countTone: 'needs-action',
      },
    ],
    selectedSavedViewId: 'reviews',
  },
}

import type { Meta, StoryObj } from '@storybook/react-vite'

import { AgentWatchNav } from '#components/agent-watch-nav'
import type { Session } from '#types'

const todaySession: Session = {
  id: 'today-session',
  category: 'needs_action',
  isPaused: false,
  kind: 'Question',
  title: 'Check the retry policy',
  preview: 'Choose a limit.',
  elapsedSeconds: 10,
  unread: true,
  task: null,
  repo: 'example/service',
  project: 'sample project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
  isToday: true,
}

const meta = {
  component: AgentWatchNav,
  args: {
    activeView: 'inbox',
    needsActionCount: 3,
    savedViews: [],
    selectedSavedViewId: null,
    workTypeNavGroups: [
      {
        label: 'Guides',
        projects: [
          {
            name: 'sample project',
            status: 'Review',
            needsActionCount: 3,
            sessionId: 'sample-guide-session',
          },
          {
            name: 'telemetry playground',
            status: 'Working',
            needsActionCount: 0,
            sessionId: 'sample-guide-session-2',
          },
          {
            name: 'handbook preview',
            status: 'Idle',
            needsActionCount: 0,
            sessionId: 'sample-guide-session-3',
          },
        ],
      },
      {
        label: 'Reviews',
        projects: [],
      },
    ],
    onViewChange: () => undefined,
    onSelectSavedView: () => undefined,
    onDeleteSavedView: () => undefined,
    todaySessions: [todaySession],
    onSelectToday: () => undefined,
    onSelectSession: () => undefined,
    onOpenPalette: () => undefined,
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

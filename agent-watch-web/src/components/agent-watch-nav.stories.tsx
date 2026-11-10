import type { Meta, StoryObj } from '@storybook/react-vite'

import { AgentWatchNav } from '#components/agent-watch-nav'
import type { Session } from '#types'

const todaySession: Session = {
  id: 'today-session',
  category: 'needs_action',
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
    todaySessions: [todaySession],
    onSelectToday: () => undefined,
    onSelectSession: () => undefined,
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

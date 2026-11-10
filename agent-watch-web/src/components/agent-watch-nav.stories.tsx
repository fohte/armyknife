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
  pullRequest: null,
  isToday: true,
}

const meta = {
  component: AgentWatchNav,
  args: {
    activeView: 'inbox',
    needsActionCount: 3,
    onViewChange: () => undefined,
    todaySessions: [todaySession],
    onSelectToday: () => undefined,
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

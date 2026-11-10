import type { Meta, StoryObj } from '@storybook/react-vite'

import { AgentWatchNav } from '#components/agent-watch-nav'

const meta = {
  component: AgentWatchNav,
  args: {
    activeView: 'inbox',
    needsActionCount: 3,
    pmProjects: [
      { name: 'service refresh', status: 'Review', needsActionCount: 3 },
      { name: 'observability', status: 'Working', needsActionCount: 0 },
      { name: 'documentation', status: 'Idle', needsActionCount: 0 },
    ],
    onViewChange: () => undefined,
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

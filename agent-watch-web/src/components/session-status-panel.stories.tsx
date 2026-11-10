import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionStatusPanel } from '#components/session-status-panel'

const meta = {
  component: SessionStatusPanel,
  args: {
    tabs: [
      {
        id: 'crit',
        label: 'crit',
        count: 1,
        content: <div className="session-status-preview">Crit review</div>,
      },
    ],
    activeTabId: 'crit',
    onTabChange: () => undefined,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '400px', height: '600px', display: 'flex' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SessionStatusPanel>

export default meta
type Story = StoryObj<typeof meta>

export const CritIsTheActiveTab: Story = {
  name: 'The panel shows crit as the active tab',
}

export const NoTabsAreAvailable: Story = {
  name: 'The panel shows an empty state when no tabs have items',
  args: {
    tabs: [{ id: 'crit', label: 'crit', count: 0, content: null }],
    activeTabId: null,
  },
}

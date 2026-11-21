import type { Meta, StoryObj } from '@storybook/react-vite'

import { MobileSheet } from '#components/mobile-sheet'

const meta = {
  component: MobileSheet,
  args: {
    label: 'Session actions',
    onClose: () => undefined,
    children: (
      <div style={{ padding: '16px' }}>
        <h2>Session actions</h2>
        <button type="button">Later</button>
      </div>
    ),
  },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        data-layout="mobile"
        style={{
          width: '390px',
          height: '844px',
          position: 'relative',
          background: 'var(--aw-bg-app)',
        }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof MobileSheet>

export default meta
type Story = StoryObj<typeof meta>

export const BottomSheetIsOpen: Story = {
  name: 'The sheet rises over the dimmed inbox',
}

export const FullScreenSheetIsOpen: Story = {
  name: 'The sheet fills the screen for a focused task',
  args: { presentation: 'fullscreen' },
}

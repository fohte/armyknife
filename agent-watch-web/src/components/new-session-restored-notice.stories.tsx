import type { Meta, StoryObj } from '@storybook/react-vite'

import { NewSessionRestoredNotice } from '#components/new-session-restored-notice'

const meta = {
  component: NewSessionRestoredNotice,
  args: {
    savedAt: '2026-05-06T12:03:00Z',
    now: new Date('2026-05-06T12:05:00Z'),
    isMobile: false,
    onClear: () => undefined,
  },
} satisfies Meta<typeof NewSessionRestoredNotice>

export default meta
type Story = StoryObj<typeof meta>

export const Desktop: Story = {
  name: 'The restored draft notice shows the save time and a Clear button',
}

export const Phone: Story = {
  name: 'On a phone the restored draft notice is taller and Clear has no border',
  args: { isMobile: true },
  decorators: [
    (Story) => (
      <div className="story-mobile-layout" style={{ width: 390 }}>
        <Story />
      </div>
    ),
  ],
}

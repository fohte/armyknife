import type { Meta, StoryObj } from '@storybook/react-vite'

import { LaterMenu } from '#components/later-menu'

const meta = {
  component: LaterMenu,
  args: {
    sessionTitle: 'Session awaiting a decision',
    isLater: true,
    laterTimes: {
      one_hour: '16:00',
      this_evening: '19:00',
      tomorrow: 'Mon 9:00',
      next_week: 'Mon 11/16',
      until_it_changes: 'next activity',
    },
    onSelect: () => {},
    onBackNow: () => {},
    onClose: () => {},
  },
} satisfies Meta<typeof LaterMenu>

export default meta
type Story = StoryObj<typeof meta>

export const MenuOffersFiveReturnTimes: Story = {
  name: 'the menu shows return times and a way back now',
}

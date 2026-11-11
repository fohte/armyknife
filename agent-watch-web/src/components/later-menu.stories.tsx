import type { Meta, StoryObj } from '@storybook/react-vite'

import { LaterMenu } from '#components/later-menu'

const meta = {
  component: LaterMenu,
  args: {
    sessionTitle: 'Session awaiting a decision',
    isLater: false,
    laterTimes: {
      one_hour: { when: '16:00', deadline: '2030-03-04T16:00:00Z' },
      this_evening: { when: '19:00', deadline: '2030-03-04T19:00:00Z' },
      tomorrow: { when: 'Mon 9:00', deadline: '2030-03-05T09:00:00Z' },
      next_week: { when: 'Mon 11/16', deadline: '2030-11-16T09:00:00Z' },
      until_it_changes: { when: 'next activity', deadline: null },
    },
    laterTimesError: false,
    isSubmitting: false,
    onSelect: () => {},
    onBackNow: () => {},
    onClose: () => {},
  },
} satisfies Meta<typeof LaterMenu>

export default meta
type Story = StoryObj<typeof meta>

export const MenuOffersFiveReturnTimes: Story = {
  name: 'The menu shows five return times for an active session',
}

export const MenuShowsBackNow: Story = {
  args: { isLater: true },
  name: 'The menu offers a way back now for a Later session',
}

export const MenuLoadsReturnTimes: Story = {
  args: {
    isLater: false,
    laterTimes: null,
    laterTimesError: false,
  },
  name: 'The menu shows placeholders while return times load',
}

export const MenuCannotLoadReturnTimes: Story = {
  args: {
    isLater: false,
    laterTimes: null,
    laterTimesError: true,
  },
  name: 'The menu disables presets when return times are unavailable',
}

export const MenuSubmitting: Story = {
  args: {
    isSubmitting: true,
  },
  name: 'The menu disables actions while a request is pending',
}

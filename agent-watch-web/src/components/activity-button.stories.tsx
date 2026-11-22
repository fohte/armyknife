import type { Meta, StoryObj } from '@storybook/react-vite'

import { ActivityButton } from '#components/activity-button'

const meta = {
  component: ActivityButton,
  args: {
    drawer: true,
    activityCount: 2,
    onOpenActivity: () => undefined,
  },
} satisfies Meta<typeof ActivityButton>

export default meta
type Story = StoryObj<typeof meta>

export const PhoneDrawerShowsRunningActions: Story = {
  name: 'The phone drawer marks two actions as running.',
  args: { isRunning: true },
}

export const NavigationKeepsActivityVisibleWhenEmpty: Story = {
  name: 'The phone drawer keeps Activity muted before any actions run.',
  args: { activityCount: 0, isRunning: false },
}

export const PhoneDrawerShowsFailedActivity: Story = {
  name: 'The mobile navigation drawer shows failed Activity results.',
  args: { failed: true },
}

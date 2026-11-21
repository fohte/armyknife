import type { Meta, StoryObj } from '@storybook/react-vite'

import { ActivityButton } from '#components/activity-button'

const meta = {
  component: ActivityButton,
  args: { activityCount: 2, onOpenActivity: () => undefined },
} satisfies Meta<typeof ActivityButton>

export default meta
type Story = StoryObj<typeof meta>

export const NavigationButtonShowsRunningActions: Story = {
  name: 'The navigation entry shows two recent actions.',
}

export const MobileDrawerShowsFailedActivity: Story = {
  name: 'The mobile navigation drawer shows failed Activity results.',
  args: { drawer: true, failed: true },
}

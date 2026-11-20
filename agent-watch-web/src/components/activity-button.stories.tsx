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

export const MobileHeaderShowsAnActivityBadge: Story = {
  name: 'The mobile header shows the action count.',
  args: { mobile: true },
}

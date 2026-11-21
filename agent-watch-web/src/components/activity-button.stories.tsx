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

export const NavigationKeepsActivityVisibleWhenEmpty: Story = {
  name: 'The navigation keeps Activity visible before any actions run.',
  args: { activityCount: 0 },
}

export const NavigationShowsOneRecentAction: Story = {
  name: 'The navigation entry shows one recent action.',
  args: { activityCount: 1 },
}

export const MobileHeaderKeepsActivityVisibleWhenEmpty: Story = {
  name: 'The mobile header keeps the Activity icon visible before any actions run.',
  args: { activityCount: 0, iconOnly: true },
  decorators: [
    (StoryComponent) => (
      <div
        style={{ display: 'flex', width: '320px', justifyContent: 'flex-end' }}
      >
        <StoryComponent />
      </div>
    ),
  ],
}

export const MobileDrawerShowsFailedActivity: Story = {
  name: 'The mobile navigation drawer shows failed Activity results.',
  args: { drawer: true, failed: true },
}

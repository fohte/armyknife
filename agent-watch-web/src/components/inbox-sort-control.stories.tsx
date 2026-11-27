import type { Decorator, Meta, StoryObj } from '@storybook/react-vite'

import { InboxSortControl } from '#components/inbox-sort-control'

const desktopDecorator: Decorator = (Story) => (
  <div style={{ width: 320, minHeight: 230, position: 'relative' }}>
    <Story />
  </div>
)

const meta = {
  component: InboxSortControl,
  args: {
    sort: 'priority',
    storageAvailable: true,
    isMobile: false,
    open: true,
    onOpenChange: () => undefined,
    onSortChange: () => undefined,
  },
} satisfies Meta<typeof InboxSortControl>

export default meta
type Story = StoryObj<typeof meta>

export const PriorityMenuIsOpen: Story = {
  name: 'The menu shows Priority selected by default',
  decorators: [desktopDecorator],
}

export const ProjectMenuIsOpen: Story = {
  name: 'The menu shows Project selected for grouped sessions',
  args: { sort: 'project' },
  decorators: [desktopDecorator],
}

export const IPhoneSortSheetIsOpen: Story = {
  name: 'The iPhone shows the current sort in a bottom sheet',
  args: { isMobile: true, sort: 'updated' },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        data-layout="mobile"
        style={{ width: '390px', height: '400px', position: 'relative' }}
      >
        <Story />
      </div>
    ),
  ],
}

export const StorageUnavailable: Story = {
  name: 'The menu explains when this browser cannot save the selection',
  args: { storageAvailable: false },
  decorators: [desktopDecorator],
}

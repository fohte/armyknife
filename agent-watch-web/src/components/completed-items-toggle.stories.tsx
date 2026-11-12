import type { Meta, StoryObj } from '@storybook/react-vite'

import { CompletedItemsToggle } from '#components/completed-items-toggle'

const meta = {
  component: CompletedItemsToggle,
  args: {
    count: 3,
    expanded: false,
    onToggle: () => undefined,
    placement: 'pull-request-list',
  },
} satisfies Meta<typeof CompletedItemsToggle>

export default meta
type Story = StoryObj<typeof meta>

export const Collapsed: Story = {
  name: 'The toggle shows the number of completed items while collapsed.',
}

export const Expanded: Story = {
  name: 'The toggle offers to hide completed items while expanded.',
  args: {
    expanded: true,
  },
}

export const LinkedTaskMenuPlacement: Story = {
  name: 'The toggle uses compact styling in the linked task menu.',
  args: {
    placement: 'linked-task-menu',
  },
}

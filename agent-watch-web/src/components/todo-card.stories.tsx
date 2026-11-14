import type { Meta, StoryObj } from '@storybook/react-vite'

import { TodoCard } from '#components/todo-card'

const meta = {
  component: TodoCard,
  args: {
    title: 'Check the updated layout on a phone',
    detail:
      'Open the preview on a small screen and confirm the action stays visible.',
    resolution: null,
    onResolve: () => Promise.resolve(true),
  },
} satisfies Meta<typeof TodoCard>

export default meta
type Story = StoryObj<typeof meta>

export const TodoWaitsForTheUser: Story = {
  name: 'A to-do waits for the user to finish it',
}

export const TodoShowsMarkdownDetail: Story = {
  name: 'A to-do detail formats links and emphasis as markdown',
  args: {
    detail:
      'Review [the example preview](https://example.test) before launch.\n\nKeep **the primary action visible** on small screens.',
  },
}

export const TodoHasNoAdditionalDetail: Story = {
  name: 'A to-do shows only its title when it has no detail',
  args: { detail: null },
}

export const TodoIsCompleted: Story = {
  name: 'A completed to-do shows its title and saved comment compactly',
  args: {
    resolution: 'completed',
    defaultComment: 'The [sample](https://example.test) is **ready**.',
  },
}

export const TodoIsCompletedWithoutAComment: Story = {
  name: 'A completed to-do without a comment shows only its title',
  args: { resolution: 'completed', defaultComment: '' },
}

export const TodoIsRejected: Story = {
  name: 'A rejected to-do shows its reason and title distinctly',
  args: { resolution: 'rejected', defaultComment: 'This is no longer needed.' },
}

export const TodoIsWithdrawn: Story = {
  name: 'A withdrawn to-do shows its title without a comment',
  args: { resolution: 'withdrawn', defaultComment: '' },
}

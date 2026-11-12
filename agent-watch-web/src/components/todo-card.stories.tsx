import type { Meta, StoryObj } from '@storybook/react-vite'

import { TodoCard } from '#components/todo-card'

const meta = {
  component: TodoCard,
  args: {
    title: 'Check the updated layout on a phone',
    detail:
      'Open the preview on a small screen and confirm the action stays visible.',
    completed: false,
    onComplete: () => Promise.resolve(true),
  },
} satisfies Meta<typeof TodoCard>

export default meta
type Story = StoryObj<typeof meta>

export const TodoWaitsForTheUser: Story = {
  name: 'A to-do waits for the user to finish it',
}

export const TodoHasNoAdditionalDetail: Story = {
  name: 'A to-do shows only its title when it has no detail',
  args: { detail: null },
}

export const TodoIsCompleted: Story = {
  name: 'A completed to-do shows its title and saved comment compactly',
  args: { completed: true, defaultComment: 'The sample is ready.' },
}

export const TodoIsCompletedWithoutAComment: Story = {
  name: 'A completed to-do without a comment shows only its title',
  args: { completed: true, defaultComment: '' },
}

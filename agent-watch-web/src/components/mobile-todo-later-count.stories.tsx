import '#components/mobile-layout.css'

import type { Meta, StoryObj } from '@storybook/react-vite'

import { MobileTodoLaterCount } from '#components/mobile-todo-later-count'

const meta = {
  component: MobileTodoLaterCount,
  args: { count: 2 },
} satisfies Meta<typeof MobileTodoLaterCount>

export default meta
type Story = StoryObj<typeof meta>

export const TwoFoldedTodos: Story = {
  name: 'The mobile row shows two deferred to-dos',
}

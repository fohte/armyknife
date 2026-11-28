import type { Meta, StoryObj } from '@storybook/react-vite'

import { WithdrawnCard } from '#components/withdrawn-card'

const meta = {
  component: WithdrawnCard,
  args: {
    kind: 'To-do',
    title: 'Review the updated guidance',
  },
} satisfies Meta<typeof WithdrawnCard>

export default meta
type Story = StoryObj<typeof meta>

export const WithdrawnTodo: Story = {
  name: 'A withdrawn to-do shows only its heading and title',
}

export const WithdrawnProposal: Story = {
  name: 'A withdrawn proposal shows only its heading and title',
  args: { kind: 'Proposal' },
}

export const WithdrawnTodoOnPhone: Story = {
  name: 'A phone shows a withdrawn to-do with a larger heading',
  parameters: { viewport: { defaultViewport: 'mobile1' } },
}

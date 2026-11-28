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

export const SessionEndedTodo: Story = {
  name: 'A to-do withdrawn when the session ended shows the end time in its heading',
  args: {
    sessionEndedAt: new Date(2026, 10, 21, 14, 32).toISOString(),
    now: new Date(2026, 10, 21),
  },
}

export const SessionEndedProposal: Story = {
  name: 'A proposal withdrawn when the session ended shows the end time in its heading',
  args: {
    kind: 'Proposal',
    sessionEndedAt: new Date(2026, 10, 21, 14, 32).toISOString(),
    now: new Date(2026, 10, 21),
  },
}

export const EarlierSessionEndedTodo: Story = {
  name: 'A to-do withdrawn when the session ended on an earlier day shows the date and time',
  args: {
    sessionEndedAt: new Date(2026, 10, 20, 14, 32).toISOString(),
    now: new Date(2026, 10, 21),
  },
}

export const SessionEndedTodoOnPhone: Story = {
  name: 'A phone shows the session end time in a withdrawn to-do heading',
  args: {
    sessionEndedAt: new Date(2026, 10, 21, 14, 32).toISOString(),
    now: new Date(2026, 10, 21),
  },
  parameters: { viewport: { defaultViewport: 'mobile1' } },
}

import type { Meta, StoryObj } from '@storybook/react-vite'

import { type Decision, DecisionCard } from '#components/decision-card'

const decision: Decision = {
  type: 'decision',
  seq: 1,
  title: 'Use a fixed retry limit',
  why: 'The current service has no reliable timing data.',
  considered: ['Retry using the latest duration', 'Retry once per package'],
  easyToChange: 'One configuration value in the CI workflow.',
  review: null,
  createdAt: '2026-11-11T03:25:59Z',
}

const meta = {
  component: DecisionCard,
  args: {
    decision,
    variant: 'conversation',
    onOk: () => Promise.resolve(true),
    onRequestChange: () => Promise.resolve(true),
  },
  decorators: [
    (Story) => (
      <div style={{ width: '720px', padding: '16px' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof DecisionCard>

export default meta
type Story = StoryObj<typeof meta>

export const ConversationRowIsCollapsed: Story = {
  name: 'The conversation keeps a decision in one collapsed row',
}

export const ConversationRowIsExpanded: Story = {
  name: 'The expanded conversation row shows the decision details',
  args: { expanded: true },
}

export const SelectedConversationRowShowsItsPosition: Story = {
  name: 'The selected conversation row shows its position in the list',
  args: { expanded: true, selected: true, position: '1 of 2' },
}

export const SelectedPanelCardIsExpanded: Story = {
  name: 'The selected panel card shows its details and review actions',
  args: {
    variant: 'panel',
    expanded: true,
    selected: true,
    targetName: 'the parent session',
  },
}

export const OtherPanelCardIsCollapsed: Story = {
  name: 'An unselected panel card shows its title and alternatives',
  args: { variant: 'panel', targetName: 'this session' },
}

export const ChangeCommentIsBeingWritten: Story = {
  name: 'The selected card has an open change request field',
  args: {
    variant: 'panel',
    expanded: true,
    selected: true,
    editing: true,
    targetName: 'the parent session',
  },
}

export const DecisionWasApproved: Story = {
  name: 'The expanded decision shows that it was approved',
  args: {
    decision: { ...decision, review: 'ok' },
    expanded: true,
  },
}

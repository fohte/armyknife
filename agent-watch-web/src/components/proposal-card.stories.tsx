import type { Meta, StoryObj } from '@storybook/react-vite'

import { ProposalCard } from '#components/proposal-card'
import type { ProposalOption } from '#types'

const options: ProposalOption[] = [
  {
    label: 'Use the existing flow',
    description: 'Keep the current sequence and add a short explanation.',
    pros: ['Small change'],
    cons: ['Adds one step'],
    recommended: true,
  },
  {
    label: 'Show the choice earlier',
    description: 'Move the decision before the next action.',
    pros: ['Clearer timing'],
    cons: ['Changes the current flow'],
    recommended: false,
  },
]

const callbacks = {
  onAnswer: () => Promise.resolve(true),
}

const meta = {
  component: ProposalCard,
  args: {
    seq: 1,
    title: 'When should the new setting take effect?',
    summary: 'Choose how the setting should be introduced.',
    options,
    answer: null,
    resolved: false,
    isMobile: false,
    ...callbacks,
  },
} satisfies Meta<typeof ProposalCard>

export default meta
type Story = StoryObj<typeof meta>

export const ProposalWaitsForAChoice: Story = {
  name: 'A proposal waits for a choice',
}

export const ProposalShowsASelectedChoice: Story = {
  name: 'A proposal marks the selected choice',
  args: { defaultChoice: 1 },
}

export const ProposalShowsACommentWithoutAChoice: Story = {
  name: 'A proposal shows a comment without a selected choice',
  args: { defaultComment: 'Consider the effect on existing users.' },
}

export const ProposalOffersTouchSizedChoicesOnMobile: Story = {
  name: 'A proposal offers touch sized choices on mobile',
  args: { isMobile: true },
}

export const ProposalShowsAChosenAnswer: Story = {
  name: 'A proposal shows the choice selected by the user',
  args: {
    answer: {
      choice: 0,
      comment: 'Keep the change small.',
    },
    resolved: true,
  },
}

export const ProposalShowsAnotherOptionWasRequested: Story = {
  name: 'A proposal shows that another option was requested',
  args: {
    answer: { choice: null, comment: 'Please include a simpler option.' },
    resolved: true,
  },
}

export const ProposalShowsItWasAnsweredElsewhere: Story = {
  name: 'A proposal shows that it was answered elsewhere',
  args: { resolved: true },
}

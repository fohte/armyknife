import type { Meta, StoryObj } from '@storybook/react-vite'

import { QuestionCard } from '#components/question-card'

const callbacks = {
  onAnswer: () => Promise.resolve(true),
  onSkip: () => Promise.resolve(true),
  onFreeformAnswer: () => Promise.resolve(true),
  onOpenInTerminal: () => undefined,
}

const meta = {
  component: QuestionCard,
  args: {
    header: 'Retry policy',
    question: 'Which **retry policy** should be used?',
    choices: [
      {
        label: 'Retry **three times** with exponential backoff',
        description: 'Wait briefly before each retry.',
      },
      { label: 'Retry once without delay' },
    ],
    multiSelect: false,
    answer: null,
    canAnswer: true,
    isMobile: false,
    showTerminalFallback: false,
    canOpenInTerminal: true,
    ...callbacks,
  },
} satisfies Meta<typeof QuestionCard>

export default meta
type Story = StoryObj<typeof meta>

export const QuestionWaitsForAnAnswer: Story = {
  name: 'A question offers choices while it waits for an answer',
}

export const QuestionShowsItsMultipleSelectType: Story = {
  name: 'A question is marked as a multiple select question',
  args: {
    multiSelect: true,
  },
}

export const QuestionShowsSelectedAnswer: Story = {
  name: 'A question marks the choice selected by the user',
  args: {
    answer: 'Retry once without delay',
    canAnswer: false,
  },
}

export const QuestionShowsFreeformAnswer: Story = {
  name: 'A question shows an answer outside its listed choices',
  args: {
    answer: 'Wait until the next deployment',
    canAnswer: false,
  },
}

export const QuestionOffersALinkedChoice: Story = {
  name: 'A choice shows its link destination as plain text',
  args: {
    choices: [
      { label: 'Follow the [setup guide](https://example.org/guide)' },
      { label: 'Use the default settings' },
    ],
  },
}

export const QuestionCanBeAnsweredInTheTerminal: Story = {
  name: 'A Codex question directs the user to the terminal',
  args: {
    canAnswer: false,
    showTerminalFallback: true,
  },
}

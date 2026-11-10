import type { Meta, StoryObj } from '@storybook/react-vite'

import { QuestionCard } from '#components/question-card'

const callbacks = {
  onAnswer: () => undefined,
  onOpenInTerminal: () => undefined,
}

const meta = {
  component: QuestionCard,
  args: {
    question: 'Which **retry policy** should be used?',
    choices: [
      'Retry **three times** with exponential backoff',
      'Retry once without delay',
    ],
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
      'Follow the [setup guide](https://example.org/guide)',
      'Use the default settings',
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

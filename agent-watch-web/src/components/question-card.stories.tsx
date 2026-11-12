import type { Meta, StoryObj } from '@storybook/react-vite'

import { QuestionCard } from '#components/question-card'

const callbacks = {
  onAnswer: () => Promise.resolve(true),
  onSkip: () => Promise.resolve(true),
  onOpenInTerminal: () => undefined,
}

const questions = [
  {
    header: 'Format',
    question: 'Which formats should be included?',
    choices: [
      { label: 'Markdown', description: 'Readable source text.' },
      { label: 'Plain text' },
      { label: 'HTML' },
    ],
    multi_select: true,
    answer: null,
  },
  {
    header: 'Examples',
    question: 'Should the guide include examples?',
    choices: [{ label: 'Yes' }, { label: 'No' }],
    multi_select: false,
    answer: null,
  },
]

const singleChoiceQuestions = [
  {
    header: 'Direction',
    question: 'Which direction should be used?',
    choices: [{ label: 'North' }, { label: 'South' }, { label: 'East' }],
    multi_select: false,
    answer: null,
  },
]

const meta = {
  component: QuestionCard,
  args: {
    questions,
    canAnswer: true,
    isMobile: false,
    showTerminalFallback: false,
    canOpenInTerminal: true,
    ...callbacks,
  },
} satisfies Meta<typeof QuestionCard>

export default meta
type Story = StoryObj<typeof meta>

export const WaitingForAllAnswers: Story = {
  name: 'A question group waits for an answer to every prompt',
  decorators: [
    (Story) => (
      <div
        style={{ width: '740px', padding: '0 24px', boxSizing: 'border-box' }}
      >
        <Story />
      </div>
    ),
  ],
}

export const WaitingOnAMobileScreen: Story = {
  name: 'A mobile question group shows its prompts in one card',
  args: {
    isMobile: true,
  },
  decorators: [
    (Story) => (
      <div
        className="conversation-pane"
        data-mobile="true"
        style={{ width: '390px', height: '844px', padding: '0 16px' }}
      >
        <Story />
      </div>
    ),
  ],
}

export const ShowingAnswers: Story = {
  name: 'A completed question group shows every recorded answer',
  args: {
    canAnswer: false,
    questions: questions.map((question, index) => ({
      ...question,
      answer: index === 0 ? 'Markdown, HTML' : 'Yes',
    })),
  },
}

export const DesktopSingleChoiceShowsShortcuts: Story = {
  name: 'A desktop single choice shows numbered shortcuts',
  args: {
    questions: singleChoiceQuestions,
    isMobile: false,
  },
}

export const MobileSingleChoiceHidesShortcuts: Story = {
  name: 'A mobile single choice shows no keyboard shortcuts',
  args: {
    questions: singleChoiceQuestions,
    isMobile: true,
  },
  decorators: [
    (Story) => (
      <div
        className="conversation-pane"
        data-mobile="true"
        style={{ width: '390px', height: '844px', padding: '0 16px' }}
      >
        <Story />
      </div>
    ),
  ],
}

export const AnsweringInTheTerminal: Story = {
  name: 'A question group without a watch-web request directs to the terminal',
  args: {
    canAnswer: false,
    showTerminalFallback: true,
  },
}

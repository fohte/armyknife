import type { Meta, StoryObj } from '@storybook/react-vite'

import { QuestionSendError } from '#components/question-send-error'

const meta = {
  component: QuestionSendError,
} satisfies Meta<typeof QuestionSendError>

export default meta
type Story = StoryObj<typeof meta>

export const AnswerCanBeRetried: Story = {
  name: 'A failed answer can be retried from its question card',
}

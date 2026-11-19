import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionCloseConfirmation } from '#components/session-close-confirmation'
import type { Session } from '#types'

const session: Session = {
  id: 'sample-session',
  category: 'in_progress',
  activityStatus: 'working',
  isPaused: false,
  kind: null,
  title: 'Implement the sample workflow',
  preview: null,
  elapsedSeconds: 42,
  unread: false,
  task: null,
  repo: 'example/service',
  project: 'service',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const meta = {
  component: SessionCloseConfirmation,
  args: {
    session,
    onCancel: () => undefined,
    onConfirm: () => undefined,
  },
} satisfies Meta<typeof SessionCloseConfirmation>

export default meta
type Story = StoryObj<typeof meta>

export const WorkingSession: Story = {
  name: 'The dialog explains that closing stops active work',
}

export const QuestionSession: Story = {
  name: 'The dialog explains that the question will stay unanswered',
  args: {
    session: {
      ...session,
      category: 'needs_action',
      kind: 'Question',
      activityStatus: 'waiting',
    },
  },
}

export const MobileSheet: Story = {
  name: 'The mobile dialog appears as a bottom sheet',
  args: { mobile: true },
}

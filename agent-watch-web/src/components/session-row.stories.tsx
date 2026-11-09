import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionRow } from '#components/session-row'
import type { Session } from '#types'

const session: Session = {
  id: 'example-session',
  category: 'needs_action',
  kind: 'Question',
  title: 'Update retry handling',
  preview: 'Should retries stop after three attempts?',
  elapsedSeconds: 8,
  unread: true,
  task: {
    id: 'example-task',
    number: 2048,
    title: 'Improve request reliability',
    priority: 1,
  },
  repo: 'example/service',
  pullRequest: null,
}

const meta = {
  component: SessionRow,
  args: {
    session,
    selected: false,
    onSelect: () => undefined,
  },
  decorators: [
    (Story) => (
      <ul className="story-session-list">
        <Story />
      </ul>
    ),
  ],
} satisfies Meta<typeof SessionRow>

export default meta
type Story = StoryObj<typeof meta>

export const NeedsActionIsUnread: Story = {
  name: 'An unread question waits for a response',
}

export const NeedsActionShowsApproval: Story = {
  name: 'An approval request waits for a response',
  args: {
    session: {
      ...session,
      kind: 'Approve',
      title: 'Allow the shell command',
      preview: 'This command needs your approval.',
    },
  },
}

export const NeedsActionShowsAReview: Story = {
  name: 'A review request waits for a response',
  args: {
    session: {
      ...session,
      kind: 'Review',
      title: 'Review the generated changes',
      preview: 'The draft is ready for review.',
    },
  },
}

export const NeedsActionShowsAPlan: Story = {
  name: 'A plan request waits for a response',
  args: {
    session: {
      ...session,
      kind: 'Plan',
      title: 'Review the implementation plan',
      preview: 'The agent is waiting for plan approval.',
    },
  },
}

export const NeedsActionShowsAnError: Story = {
  name: 'An error request uses the red status color',
  args: {
    session: {
      ...session,
      kind: 'Error',
      title: 'Retry limit was reached',
      preview: 'The request failed after the final retry.',
    },
  },
}

export const NeedsActionShowsAStoppedSession: Story = {
  name: 'A stopped session uses the muted status color',
  args: {
    session: {
      ...session,
      kind: 'Stopped',
      title: 'Session stopped unexpectedly',
      unread: false,
    },
  },
}

export const WorkIsInProgress: Story = {
  name: 'An active session shows its working time',
  args: {
    session: {
      ...session,
      category: 'in_progress',
      kind: null,
      unread: false,
      elapsedSeconds: 30,
    },
  },
}

export const WorkIsDone: Story = {
  name: 'A completed session shows its merged pull request',
  args: {
    session: {
      ...session,
      category: 'done',
      kind: null,
      unread: false,
      pullRequest: { number: 2049, state: 'Merged' },
      elapsedSeconds: 3600,
    },
  },
}

import type { Meta, StoryObj } from '@storybook/react-vite'

import { PullRequestMergeFeedback } from '#components/pull-request-merge-feedback'

const meta = {
  component: PullRequestMergeFeedback,
  args: {
    kind: 'success',
    message: 'PR #42 merged.',
    isMobile: true,
  },
  decorators: [
    (Story) => (
      <div className="story-mobile-layout">
        <div className="conversation-pane" data-mobile="true">
          <header className="conversation-header has-session-pr-feedback">
            <div style={{ gridColumn: '1 / -1', gridRow: 1 }}>
              Mobile header actions
            </div>
            <div style={{ gridColumn: '1 / -1', gridRow: 2 }}>
              example/service
            </div>
            <div style={{ gridColumn: '1 / -1', gridRow: 3 }}>
              PR #42 · Open
            </div>
            <Story />
          </header>
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof PullRequestMergeFeedback>

export default meta
type Story = StoryObj<typeof meta>

export const MobileMergeSucceeded: Story = {
  name: 'The mobile header announces that a pull request merged.',
}

export const MobileMergeWasQueued: Story = {
  name: 'The mobile header announces that a pull request entered the queue.',
  args: {
    kind: 'queued',
    message: 'PR #42 was added to the merge queue: checks are pending.',
  },
}

export const MobileMergeFailed: Story = {
  name: 'The mobile header reports a pull request merge error.',
  args: {
    kind: 'error',
    message: 'The merge request could not be completed.',
  },
}

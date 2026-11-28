import type { Meta, StoryObj } from '@storybook/react-vite'

import { NewSessionStarting } from '#components/new-session-starting'
import { exampleSessionDirectories } from '#new-session-story-fixtures.test'

const meta = {
  component: NewSessionStarting,
  args: {
    isMobile: false,
    now: new Date('2026-05-06T12:03:00Z'),
    pendingLaunch: {
      request: {
        directory: exampleSessionDirectories[0]?.path ?? '',
        engine: 'claude',
        worktree: false,
        initialMessage: 'Add a setting for retaining application logs.',
      },
      startedAt: new Date('2026-05-06T12:03:00Z').getTime(),
    },
  },
  decorators: [
    (Story) => (
      <div style={{ display: 'flex', width: '100%', height: 650 }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof NewSessionStarting>

export default meta
type Story = StoryObj<typeof meta>

export const SentMessageShowsWhileStarting: Story = {
  name: 'The sent first message shows above the line that Claude Code is starting',
}

export const IPhoneShowsMessageAndReplyField: Story = {
  name: 'On iPhone the sent first message shows above the starting line with an empty reply field',
  args: { isMobile: true },
  decorators: [
    (Story) => (
      <div
        className="agent-watch-shell"
        data-layout="mobile"
        data-mobile-screen="conversation"
        style={{ display: 'flex', width: 390, height: 844 }}
      >
        <Story />
      </div>
    ),
  ],
}

export const EmptyMessageShowsOnlyStartingLine: Story = {
  name: 'Without a first message only the starting line shows',
  args: {
    pendingLaunch: {
      request: {
        directory: exampleSessionDirectories[0]?.path ?? '',
        engine: 'claude',
        worktree: false,
      },
      startedAt: new Date('2026-05-06T12:03:00Z').getTime(),
    },
  },
}

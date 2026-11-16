import type { Meta, StoryObj } from '@storybook/react-vite'

import { NewSessionPicker } from '#components/new-session-picker'
import {
  createNewSessionStoryState,
  exampleSessionDirectories,
} from '#new-session-story-fixtures.test'

const meta = {
  component: NewSessionPicker,
  args: {
    state: createNewSessionStoryState(),
    onClose: () => undefined,
    onLaunched: () => undefined,
  },
} satisfies Meta<typeof NewSessionPicker>

export default meta
type Story = StoryObj<typeof meta>

export const DirectorySelectionIsOpen: Story = {
  name: 'The picker shows directories before engine choices',
}

export const EngineAndWorktreeCanBeSelected: Story = {
  name: 'Claude Code, Codex, and worktree options appear for the chosen directory',
  args: {
    state: createNewSessionStoryState({
      selectedDirectory: exampleSessionDirectories[0] ?? null,
      engine: 'codex',
      worktree: true,
      initialMessage: 'Add a setting for retaining application logs.',
    }),
  },
}

export const IPhoneShowsAStartButton: Story = {
  name: 'The picker adds a full-width start button on an iPhone',
  args: {
    isMobile: true,
    state: createNewSessionStoryState({
      selectedDirectory: exampleSessionDirectories[1] ?? null,
    }),
  },
  decorators: [
    (Story) => (
      <div className="story-mobile-layout" style={{ height: '844px' }}>
        <Story />
      </div>
    ),
  ],
}

export const StartedSessionIsPendingInTheFeed: Story = {
  name: 'The picker waits until the started session appears in the feed',
  args: {
    launchPending: true,
    state: createNewSessionStoryState({
      selectedDirectory: exampleSessionDirectories[0] ?? null,
    }),
  },
}

export const LaunchRequestIsSending: Story = {
  name: 'The picker shows progress while the launch request is sending',
  args: {
    state: createNewSessionStoryState({
      selectedDirectory: exampleSessionDirectories[0] ?? null,
      isStarting: true,
    }),
  },
}

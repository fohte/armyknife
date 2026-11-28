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
    now: new Date('2026-05-06T12:00:00Z'),
    onClose: () => undefined,
    onStart: () => undefined,
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

export const RestoredDraft: Story = {
  name: 'The palette shows the restored draft notice under the input',
  args: {
    state: createNewSessionStoryState({
      selectedDirectory: exampleSessionDirectories[0] ?? null,
      initialMessage: 'Resume the unfinished setup.',
      restoredAt: '2026-05-06T12:03:00Z',
    }),
  },
}

export const RestoredDraftBeforeDirectorySelection: Story = {
  name: 'The palette shows a restored notice below directory search',
  args: {
    state: createNewSessionStoryState({
      directoryQuery: 'project',
      restoredAt: '2026-05-06T12:03:00Z',
    }),
  },
}

export const ModelEffortOptionsAreOpen: Story = {
  name: 'The command palette exposes the selected model and effort controls',
  args: {
    initialModelPickerOpen: true,
    state: createNewSessionStoryState({
      selectedDirectory: exampleSessionDirectories[0] ?? null,
      selectedModel: 'fixture-model-comet',
      selectedEffort: 'high',
      activeModelOption: {
        id: 'fixture-model-comet',
        label: 'Comet',
        defaultEffort: 'high',
        efforts: ['low', 'high'],
      },
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

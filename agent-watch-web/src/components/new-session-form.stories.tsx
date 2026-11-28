import type { Meta, StoryObj } from '@storybook/react-vite'

import { NewSessionForm } from '#components/new-session-form'
import {
  createNewSessionStoryState,
  exampleSessionDirectories,
} from '#new-session-story-fixtures.test'

const meta = {
  component: NewSessionForm,
  args: {
    state: createNewSessionStoryState(),
    now: new Date('2026-05-06T12:00:00Z'),
    onClose: () => undefined,
    onStart: () => undefined,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '100%', height: '700px' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof NewSessionForm>

export default meta
type Story = StoryObj<typeof meta>

export const DirectoryIsRequired: Story = {
  name: 'Start stays disabled until a directory is selected',
}

export const DirectoryPickerIsOpen: Story = {
  name: 'The directory picker groups recent paths and projects',
  args: {
    initialDirectoryPickerOpen: true,
    state: createNewSessionStoryState({
      isLoadingDirectories: false,
      directories: exampleSessionDirectories,
    }),
  },
}

export const FirstMessageAndWorktreeAreSelected: Story = {
  name: 'A selected directory can start Codex in a new worktree',
  args: {
    state: createNewSessionStoryState({
      selectedDirectory: exampleSessionDirectories[0] ?? null,
      engine: 'codex',
      worktree: true,
      initialMessage: 'Add a setting for retaining application logs.',
    }),
  },
}

export const ModelAndEffortPickerIsOpen: Story = {
  name: 'The selected model shows its available effort levels',
  args: {
    initialModelPickerOpen: true,
    state: createNewSessionStoryState({
      selectedDirectory: exampleSessionDirectories[0] ?? null,
      selectedModel: 'fixture-model-orbit',
      selectedEffort: 'high',
      activeModelOption: {
        id: 'fixture-model-orbit',
        label: 'Orbit',
        defaultEffort: 'medium',
        efforts: ['low', 'medium', 'high'],
      },
    }),
  },
}

export const SessionIsStarting: Story = {
  name: 'The form waits for the launched session to appear in the feed',
  args: {
    launchPending: true,
    state: createNewSessionStoryState({
      selectedDirectory: exampleSessionDirectories[0] ?? null,
    }),
  },
}

export const IPhoneShowsTheEmptyNewSession: Story = {
  name: 'The empty session form fills an iPhone screen',
  args: { isMobile: true },
  decorators: [
    (Story) => (
      <div
        className="agent-watch-shell"
        data-layout="mobile"
        data-mobile-screen="conversation"
        style={{ width: 390, height: 844 }}
      >
        <Story />
      </div>
    ),
  ],
}

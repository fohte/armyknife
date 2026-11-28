import type { SessionDirectory, SessionModelOptions } from '#new-session-api'
import type { NewSessionState } from '#use-new-session'

export const exampleModelOptions: SessionModelOptions = {
  defaultModel: 'fixture-model-orbit',
  defaultEffort: 'medium',
  models: [
    {
      id: 'fixture-model-orbit',
      label: 'Orbit',
      defaultEffort: 'medium',
      efforts: ['low', 'medium', 'high'],
    },
    {
      id: 'fixture-model-comet',
      label: 'Comet',
      defaultEffort: 'high',
      efforts: ['low', 'high'],
    },
  ],
}

export const exampleSessionDirectories: SessionDirectory[] = [
  {
    path: '/Users/example/work/service',
    source: 'recent',
    lastUsedAt: '2026-05-06T10:00:00Z',
  },
  { path: '/Users/example/work/handbook', source: 'repo' },
  { path: '/Users/example/work/scratch', source: 'path' },
]

export function createNewSessionStoryState(
  overrides: Partial<NewSessionState> = {},
): NewSessionState {
  return {
    directoryQuery: '',
    directories: exampleSessionDirectories,
    selectedDirectory: null,
    engine: 'claude',
    modelOptions: exampleModelOptions,
    isLoadingModelOptions: false,
    modelOptionsError: null,
    selectedModel: null,
    selectedEffort: null,
    modelChangeNotice: null,
    activeModelOption: exampleModelOptions.models[0],
    worktree: false,
    initialMessage: '',
    skillSuggestions: [],
    isLoadingDirectories: false,
    isStarting: false,
    error: null,
    setDirectoryQuery: () => undefined,
    chooseDirectory: () => undefined,
    setEngine: () => undefined,
    chooseModel: () => undefined,
    setSelectedEffort: () => undefined,
    undoEngineChange: () => undefined,
    setWorktree: () => undefined,
    setInitialMessage: () => undefined,
    start: () => Promise.resolve(),
    reset: () => undefined,
    ...overrides,
  }
}

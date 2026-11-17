import type { SessionDirectory } from '#new-session-api'
import type { NewSessionState } from '#use-new-session'

export const exampleSessionDirectories: SessionDirectory[] = [
  { path: '/Users/example/work/service', source: 'recent' },
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
    worktree: false,
    initialMessage: '',
    skillSuggestions: [],
    isLoadingDirectories: false,
    isStarting: false,
    error: null,
    setDirectoryQuery: () => undefined,
    chooseDirectory: () => undefined,
    setEngine: () => undefined,
    setWorktree: () => undefined,
    setInitialMessage: () => undefined,
    start: () => Promise.resolve(),
    reset: () => undefined,
    ...overrides,
  }
}

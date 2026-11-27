import { expect, it } from 'vitest'

import {
  isSelectedSessionBeyondVisibleInProgress,
  readInboxSort,
  sortInboxSessions,
  sortInboxSessionsByCategory,
  writeInboxSort,
} from '#inbox-sort'
import { moveSelection } from '#session-navigation'
import type { Session } from '#types'

const storageKey = 'agent-watch-web:inbox-sort'

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>()

  get length() {
    return this.values.size
  }

  clear() {
    this.values.clear()
  }

  getItem(key: string) {
    return this.values.get(key) ?? null
  }

  key(index: number) {
    return [...this.values.keys()][index] ?? null
  }

  removeItem(key: string) {
    this.values.delete(key)
  }

  setItem(key: string, value: string) {
    this.values.set(key, value)
  }
}

class UnavailableStorage extends MemoryStorage {
  override getItem(key: string): string | null {
    throw new Error(`Storage is blocked: ${key}`)
  }

  override setItem(key: string, value: string): void {
    throw new Error(`Storage is blocked: ${key}=${value}`)
  }
}

function session(
  id: string,
  project: string,
  priority: number | null,
  updatedAt: string,
): Session {
  return {
    id,
    category: 'needs_action',
    isPaused: false,
    kind: null,
    title: id,
    preview: null,
    elapsedSeconds: 0,
    unread: false,
    task:
      priority === null
        ? null
        : { id: `task-${id}`, number: 1, title: id, priority },
    repo: project,
    project,
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
    updatedAt,
  }
}

const sessions = [
  session('sample-zeta-low', 'zeta', 3, '2030-01-01T10:00:00.000Z'),
  session('sample-alpha-late', 'alpha', 1, '2030-01-01T12:00:00.000Z'),
  session('sample-alpha-early', 'alpha', 1, '2030-01-01T11:00:00.000Z'),
  session('sample-zeta-high', 'zeta', 1, '2030-01-01T09:00:00.000Z'),
]

it.each([
  {
    sort: 'priority' as const,
    expected: [
      'sample-zeta-low',
      'sample-alpha-late',
      'sample-alpha-early',
      'sample-zeta-high',
    ],
  },
  {
    sort: 'updated' as const,
    expected: [
      'sample-alpha-late',
      'sample-alpha-early',
      'sample-zeta-low',
      'sample-zeta-high',
    ],
  },
  {
    sort: 'project' as const,
    expected: [
      'sample-alpha-late',
      'sample-alpha-early',
      'sample-zeta-low',
      'sample-zeta-high',
    ],
  },
])(
  'sorts sessions by $sort without changing the input',
  ({ sort, expected }) => {
    const result = sortInboxSessions(sessions, sort)

    const snapshot = () => ({
      result: result.map(({ id }) => id),
      input: sessions.map(({ id }) => id),
    })
    expect(snapshot()).toEqual({
      result: expected,
      input: [
        'sample-zeta-low',
        'sample-alpha-late',
        'sample-alpha-early',
        'sample-zeta-high',
      ],
    })
  },
)

it('preserves the server order for Priority regardless of numeric priority', () => {
  expect(
    sortInboxSessions(
      [
        session(
          'sample-unprioritized',
          'alpha',
          null,
          '2030-01-01T12:00:00.000Z',
        ),
        session('sample-prioritized', 'alpha', 2, '2030-01-01T09:00:00.000Z'),
      ],
      'priority',
    ).map(({ id }) => id),
  ).toEqual(['sample-unprioritized', 'sample-prioritized'])
})

it.each(['priority', 'updated', 'project'] as const)(
  'preserves input order for sessions with matching %s keys',
  (sort) => {
    const tiedSessions = [
      session('sample-first', 'sample-project', 1, '2030-01-01T12:00:00.000Z'),
      session('sample-second', 'sample-project', 1, '2030-01-01T12:00:00.000Z'),
    ]

    expect(sortInboxSessions(tiedSessions, sort)).toEqual(tiedSessions)
  },
)

it('places sessions without update timestamps after timestamped sessions', () => {
  const missingUpdatedAtFirst = session(
    'sample-missing-updated-at-first',
    'sample-project',
    null,
    '2030-01-01T10:00:00.000Z',
  )
  const missingUpdatedAtSecond = session(
    'sample-missing-updated-at-second',
    'sample-project',
    null,
    '2030-01-01T10:00:00.000Z',
  )
  delete missingUpdatedAtFirst.updatedAt
  delete missingUpdatedAtSecond.updatedAt

  expect(
    sortInboxSessions(
      [
        missingUpdatedAtFirst,
        session(
          'sample-timestamped',
          'sample-project',
          null,
          '2030-01-01T12:00:00.000Z',
        ),
        missingUpdatedAtSecond,
      ],
      'updated',
    ).map(({ id }) => id),
  ).toEqual([
    'sample-timestamped',
    'sample-missing-updated-at-first',
    'sample-missing-updated-at-second',
  ])
})

it.each([
  {
    sort: 'updated' as const,
    expected: [
      'sample-needs-action-new',
      'sample-needs-action-old',
      'sample-progress-new',
      'sample-progress-old',
    ],
  },
  {
    sort: 'project' as const,
    expected: [
      'sample-needs-action-old',
      'sample-needs-action-new',
      'sample-progress-new',
      'sample-progress-old',
    ],
  },
])(
  'moves j and k in the visible category order for $sort',
  ({ sort, expected }) => {
    const sessions: Session[] = [
      {
        ...session(
          'sample-needs-action-new',
          'zeta',
          null,
          '2030-01-01T12:00:00.000Z',
        ),
        category: 'needs_action',
      },
      {
        ...session(
          'sample-needs-action-old',
          'alpha',
          null,
          '2030-01-01T10:00:00.000Z',
        ),
        category: 'needs_action',
      },
      {
        ...session(
          'sample-progress-new',
          'alpha',
          null,
          '2030-01-01T12:00:00.000Z',
        ),
        category: 'in_progress',
      },
      {
        ...session(
          'sample-progress-old',
          'zeta',
          null,
          '2030-01-01T10:00:00.000Z',
        ),
        category: 'in_progress',
      },
    ]
    const sorted = sortInboxSessionsByCategory(sessions, sort)

    const navigationSnapshot = () => ({
      ids: sorted.map(({ id }) => id),
      nextAfterFirst: moveSelection(sorted, expected[0] ?? null, 'j'),
      previousBeforeThird: moveSelection(sorted, expected[2] ?? null, 'k'),
    })
    expect(navigationSnapshot()).toEqual({
      ids: expected,
      nextAfterFirst: expected[1],
      previousBeforeThird: expected[1],
    })
  },
)

it.each([
  {
    sort: 'updated' as const,
    selectedId: 'sample-progress-zeta',
    expected: true,
  },
  {
    sort: 'project' as const,
    selectedId: 'sample-progress-zeta',
    expected: true,
  },
])(
  'detects a selected In progress row below the visible first two for $sort',
  ({ sort, selectedId, expected }) => {
    const inProgressSessions: Session[] = [
      {
        ...session(
          'sample-progress-zeta',
          'zeta',
          null,
          '2030-01-01T10:00:00.000Z',
        ),
        category: 'in_progress',
      },
      {
        ...session(
          'sample-progress-beta',
          'beta',
          null,
          '2030-01-01T12:00:00.000Z',
        ),
        category: 'in_progress',
      },
      {
        ...session(
          'sample-progress-alpha',
          'alpha',
          null,
          '2030-01-01T11:00:00.000Z',
        ),
        category: 'in_progress',
      },
    ]

    expect(
      isSelectedSessionBeyondVisibleInProgress(
        inProgressSessions,
        selectedId,
        sort,
      ),
    ).toEqual(expected)
  },
)

it('reads and writes a browser-specific sort selection', () => {
  const storage = new MemoryStorage()
  const written = writeInboxSort(storage, 'updated')

  const snapshot = () => ({
    written,
    stored: storage.getItem(storageKey),
    restored: readInboxSort(storage),
  })
  expect(snapshot()).toEqual({
    written: true,
    stored: 'updated',
    restored: { sort: 'updated', storageAvailable: true },
  })
})

it.each([
  { value: null, label: 'missing' },
  { value: 'unknown', label: 'invalid' },
])('defaults to Priority for $label stored selections', ({ value }) => {
  const storage = new MemoryStorage()
  if (value !== null) storage.setItem(storageKey, value)

  expect(readInboxSort(storage)).toEqual({
    sort: 'priority',
    storageAvailable: true,
  })
})

it('reports when browser storage is unavailable', () => {
  const storage = new UnavailableStorage()

  const snapshot = () => ({
    read: readInboxSort(storage),
    written: writeInboxSort(storage, 'updated'),
  })

  expect(snapshot()).toEqual({
    read: { sort: 'priority', storageAvailable: false },
    written: false,
  })
})

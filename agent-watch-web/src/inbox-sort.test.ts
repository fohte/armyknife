import { expect, it } from 'vitest'

import {
  DEFAULT_INBOX_SORT,
  INBOX_SORT_STORAGE_KEY,
  readInboxSort,
  sortInboxSessions,
  writeInboxSort,
} from '#inbox-sort'
import type { Session } from '#types'

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
      'sample-alpha-late',
      'sample-alpha-early',
      'sample-zeta-high',
      'sample-zeta-low',
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
      'sample-zeta-high',
      'sample-zeta-low',
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

it('places sessions without a priority after prioritized sessions', () => {
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
  ).toEqual(['sample-prioritized', 'sample-unprioritized'])
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

it('reads and writes a browser-specific sort selection', () => {
  const storage = new MemoryStorage()
  const written = writeInboxSort(storage, 'updated')

  const snapshot = () => ({
    written,
    stored: storage.getItem(INBOX_SORT_STORAGE_KEY),
    restored: readInboxSort(storage),
  })
  expect(snapshot()).toEqual({
    written: true,
    stored: 'updated',
    restored: 'updated',
  })
})

it.each([
  { value: null, label: 'missing' },
  { value: 'unknown', label: 'invalid' },
])('defaults to Priority for $label stored selections', ({ value }) => {
  const storage = new MemoryStorage()
  if (value !== null) storage.setItem(INBOX_SORT_STORAGE_KEY, value)

  expect(readInboxSort(storage)).toBe(DEFAULT_INBOX_SORT)
})

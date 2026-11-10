import { expect, it } from 'vitest'

import {
  buildCommandPaletteGroups,
  readRecentSessionIds,
  recordRecentSessionId,
} from '#command-palette-model'
import type { Session } from '#types'

function session(
  id: string,
  title: string,
  category: Session['category'],
  task: Session['task'],
  project: string,
): Session {
  return {
    id,
    category,
    isPaused: false,
    kind: 'Question',
    title,
    preview: null,
    elapsedSeconds: 120,
    unread: false,
    task,
    repo: 'example/service',
    project,
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  }
}

const task = { id: 'task-a', number: 42, title: 'Improve alerts', priority: 1 }
const sessions = [
  session('session-a', 'Review alert copy', 'needs_action', task, 'alerts'),
  session('session-b', 'Add alert settings', 'in_progress', task, 'alerts'),
  session('session-c', 'Update readme', 'done', null, 'docs'),
]

it('builds default groups from needs-action, recent, and available commands', () => {
  expect(
    buildCommandPaletteGroups(sessions, '', 'all', ['session-b'], false),
  ).toEqual([
    {
      heading: 'NEEDS ACTION',
      items: [{ type: 'session', id: 'session-a', session: sessions[0] }],
    },
    {
      heading: 'RECENT',
      items: [{ type: 'session', id: 'session-b', session: sessions[1] }],
    },
    {
      heading: 'COMMANDS',
      items: [
        {
          type: 'command',
          id: 'switch-board',
          label: 'Switch to Board',
          description: 'Show projects and sessions',
        },
        {
          type: 'command',
          id: 'switch-inbox',
          label: 'Switch to Inbox',
          description: 'Show the session inbox',
        },
        {
          type: 'command',
          id: 'archive-done',
          label: 'Archive all done',
          description: 'Archive every done session',
        },
        {
          type: 'command',
          id: 'filter-task:42',
          label: 'Filter inbox by task #42',
          description: 'Improve alerts',
          taskNumber: 42,
        },
      ],
    },
  ])
})

it('matches text and scopes task results from a leading prefix', () => {
  expect(
    buildCommandPaletteGroups(sessions, '#alert', 'all', [], false),
  ).toEqual([
    {
      heading: 'TQ TASKS',
      items: [
        {
          type: 'task',
          id: 'task:task-a',
          task,
          sessions: [sessions[0], sessions[1]],
        },
      ],
    },
  ])
})

it('records a session once at the front of the recent list', () => {
  const values = new Map<string, string>()
  const storage: Storage = {
    length: 0,
    clear: () => {
      values.clear()
    },
    getItem: (key: string) => values.get(key) ?? null,
    key: () => null,
    removeItem: (key: string) => {
      values.delete(key)
    },
    setItem: (key: string, value: string) => {
      values.set(key, value)
    },
  }
  recordRecentSessionId(storage, 'session-a')
  recordRecentSessionId(storage, 'session-b')
  recordRecentSessionId(storage, 'session-a')

  expect(readRecentSessionIds(storage)).toEqual(['session-a', 'session-b'])
})

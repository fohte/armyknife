import { expect, it } from 'vitest'

import {
  filterInboxSessions,
  getInboxFilterOptions,
  parseInboxFilterQuery,
} from '#inbox-filter'
import type { Session } from '#types'

const sessions: Session[] = [
  {
    id: 'sample-question',
    category: 'needs_action',
    isPaused: false,
    kind: 'Question',
    engine: 'Claude',
    title: 'Check the request retry policy',
    preview: null,
    elapsedSeconds: 10,
    unread: true,
    task: {
      id: 'sample-task',
      number: 123,
      title: 'Improve request reliability',
      priority: 1,
    },
    repo: 'sample-org/service',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'sample-review',
    category: 'later',
    isPaused: false,
    kind: 'Review',
    engine: 'Codex',
    title: 'Review service changes',
    preview: null,
    elapsedSeconds: 20,
    unread: false,
    task: null,
    repo: 'sample-org/service',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'sample-error',
    category: 'done',
    isPaused: false,
    kind: 'Error',
    engine: 'Codex',
    title: 'Inspect the build error',
    preview: null,
    elapsedSeconds: 30,
    unread: false,
    task: {
      id: 'other-task',
      number: 123,
      title: 'Inspect build output',
      priority: null,
    },
    repo: 'sample-org/worker',
    project: 'api',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
]

it.each([
  {
    query: 'project:web-app kind:review,question task:#123',
    expected: [
      { key: 'project', values: ['web-app'] },
      { key: 'kind', values: ['review', 'question'] },
      { key: 'task', values: ['#123'] },
    ],
  },
  {
    query: 'REPO:sample-org/service engine:Codex',
    expected: [
      { key: 'repo', values: ['sample-org/service'] },
      { key: 'engine', values: ['Codex'] },
    ],
  },
  { query: 'kind: , project:', expected: [] },
])('parses inbox filter query $query', ({ query, expected }) => {
  expect(parseInboxFilterQuery(query)).toEqual(expected)
})

it('applies OR values within a field and AND between fields without case sensitivity', () => {
  const filters = parseInboxFilterQuery('project:WEB-APP kind:question,REVIEW')

  expect(filterInboxSessions(sessions, filters)).toEqual(sessions.slice(0, 2))
})

it('matches task numbers with or without the hash prefix and combines fields with AND', () => {
  const filters = parseInboxFilterQuery(
    'project:web-app kind:question task:123 repo:SAMPLE-ORG/SERVICE engine:claude',
  )

  expect(filterInboxSessions(sessions, filters)).toEqual([sessions[0]])
})

it('counts each value after applying the other selected filters', () => {
  const filters = parseInboxFilterQuery('project:web-app kind:question,review')

  expect(getInboxFilterOptions(sessions, filters, 'kind')).toEqual([
    { value: 'Question', count: 1 },
    { value: 'Review', count: 1 },
  ])
})

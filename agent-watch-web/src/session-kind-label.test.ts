import { expect, it } from 'vitest'

import { sessionKindLabel } from '#session-kind-label'
import type { Session } from '#types'

const baseSession: Session = {
  id: 'sample-session',
  category: 'needs_action',
  isPaused: false,
  kind: null,
  title: 'Sample session',
  preview: null,
  elapsedSeconds: 0,
  unread: false,
  task: null,
  repo: 'sample/repository',
  project: 'sample project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

it.each([
  {
    label: 'kind labels take precedence',
    session: { kind: 'Review' },
    expected: 'Review',
  },
  {
    label: 'needs action has a fallback label',
    session: { kind: null },
    expected: 'Needs action',
  },
  {
    label: 'in progress has a fallback label',
    session: { category: 'in_progress', kind: null },
    expected: 'Working',
  },
  {
    label: 'later has a fallback label',
    session: { category: 'later', kind: null },
    expected: 'Later',
  },
  {
    label: 'done has a fallback label',
    session: { category: 'done', kind: null },
    expected: 'Done',
  },
] satisfies {
  label: string
  session: Partial<Session>
  expected: string
}[])('returns $expected when $label', ({ session, expected }) => {
  expect(sessionKindLabel({ ...baseSession, ...session })).toEqual(expected)
})

import { expect, it } from 'vitest'

import {
  type BoardProject,
  buildBoardProjects,
  sortPmProjects,
} from '#board-model'
import type { Session } from '#types'

function session(
  id: string,
  project: string,
  category: Session['category'],
): Session {
  return {
    id,
    category,
    kind: category === 'needs_action' ? 'Question' : null,
    title: id,
    preview: null,
    elapsedSeconds: 8,
    unread: false,
    task: null,
    repo: 'example/' + project,
    project,
    ancestorSessionIds: [],
    parent: null,
    pullRequest: null,
  }
}

it('sorts projects by action count, activity, then name', () => {
  const sessions = [
    session('idle-z', 'zeta', 'done'),
    session('working-b', 'beta', 'in_progress'),
    session('action-a', 'alpha', 'needs_action'),
    session('action-a-2', 'alpha', 'needs_action'),
    session('action-c', 'charlie', 'needs_action'),
    session('working-b-2', 'beta', 'in_progress'),
  ] as const

  const expected: BoardProject[] = [
    {
      name: 'alpha',
      sessions: [sessions[2], sessions[3]],
      repos: ['example/alpha'],
      needsActionCount: 2,
      inProgressCount: 0,
      quiet: false,
      status: 'Review',
    },
    {
      name: 'charlie',
      sessions: [sessions[4]],
      repos: ['example/charlie'],
      needsActionCount: 1,
      inProgressCount: 0,
      quiet: false,
      status: 'Review',
    },
    {
      name: 'beta',
      sessions: [sessions[1], sessions[5]],
      repos: ['example/beta'],
      needsActionCount: 0,
      inProgressCount: 2,
      quiet: false,
      status: 'Working',
    },
    {
      name: 'zeta',
      sessions: [sessions[0]],
      repos: ['example/zeta'],
      needsActionCount: 0,
      inProgressCount: 0,
      quiet: true,
      status: 'Idle',
    },
  ] as const

  expect(buildBoardProjects(sessions)).toEqual(expected)
})

it('hides only quiet projects when the quiet filter is enabled', () => {
  const sessions = [
    session('action', 'alpha', 'needs_action'),
    session('working', 'beta', 'in_progress'),
    session('quiet', 'gamma', 'done'),
  ] as const

  const expected: BoardProject[] = [
    {
      name: 'alpha',
      sessions: [sessions[0]],
      repos: ['example/alpha'],
      needsActionCount: 1,
      inProgressCount: 0,
      quiet: false,
      status: 'Review',
    },
    {
      name: 'beta',
      sessions: [sessions[1]],
      repos: ['example/beta'],
      needsActionCount: 0,
      inProgressCount: 1,
      quiet: false,
      status: 'Working',
    },
    {
      name: 'gamma',
      sessions: [sessions[2]],
      repos: ['example/gamma'],
      needsActionCount: 0,
      inProgressCount: 0,
      quiet: true,
      status: 'Idle',
    },
  ]

  expect(buildBoardProjects(sessions)).toEqual(expected)
})

it('orders PM projects by status before activity and name', () => {
  const sessions = [
    session('idle', 'alpha', 'done'),
    session('working', 'bravo', 'in_progress'),
    session('review-b', 'charlie', 'needs_action'),
    session('review-a', 'delta', 'needs_action'),
  ] as const

  const projects = buildBoardProjects(sessions)
  const expected: BoardProject[] = [
    {
      name: 'charlie',
      sessions: [sessions[2]],
      repos: ['example/charlie'],
      needsActionCount: 1,
      inProgressCount: 0,
      quiet: false,
      status: 'Review',
    },
    {
      name: 'delta',
      sessions: [sessions[3]],
      repos: ['example/delta'],
      needsActionCount: 1,
      inProgressCount: 0,
      quiet: false,
      status: 'Review',
    },
    {
      name: 'bravo',
      sessions: [sessions[1]],
      repos: ['example/bravo'],
      needsActionCount: 0,
      inProgressCount: 1,
      quiet: false,
      status: 'Working',
    },
    {
      name: 'alpha',
      sessions: [sessions[0]],
      repos: ['example/alpha'],
      needsActionCount: 0,
      inProgressCount: 0,
      quiet: true,
      status: 'Idle',
    },
  ]

  expect(sortPmProjects(projects)).toEqual(expected)
})

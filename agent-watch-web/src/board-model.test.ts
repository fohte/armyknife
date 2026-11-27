import { expect, it } from 'vitest'

import {
  type BoardProject,
  buildBoardProjects,
  buildWorkTypeNavGroups,
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
    isPaused: false,
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
      waitingCount: 0,
      inProgressCount: 0,
      quiet: false,
      status: 'Review',
    },
    {
      name: 'charlie',
      sessions: [sessions[4]],
      repos: ['example/charlie'],
      needsActionCount: 1,
      waitingCount: 0,
      inProgressCount: 0,
      quiet: false,
      status: 'Review',
    },
    {
      name: 'beta',
      sessions: [sessions[1], sessions[5]],
      repos: ['example/beta'],
      needsActionCount: 0,
      waitingCount: 0,
      inProgressCount: 2,
      quiet: false,
      status: 'Working',
    },
    {
      name: 'zeta',
      sessions: [sessions[0]],
      repos: ['example/zeta'],
      needsActionCount: 0,
      waitingCount: 0,
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
      waitingCount: 0,
      inProgressCount: 0,
      quiet: false,
      status: 'Review',
    },
    {
      name: 'beta',
      sessions: [sessions[1]],
      repos: ['example/beta'],
      needsActionCount: 0,
      waitingCount: 0,
      inProgressCount: 1,
      quiet: false,
      status: 'Working',
    },
    {
      name: 'gamma',
      sessions: [sessions[2]],
      repos: ['example/gamma'],
      needsActionCount: 0,
      waitingCount: 0,
      inProgressCount: 0,
      quiet: true,
      status: 'Idle',
    },
  ]

  expect(buildBoardProjects(sessions)).toEqual(expected)
})

it('keeps a waiting-only project active and orders it after working projects', () => {
  const sessions = [
    session('waiting', 'beta', 'waiting'),
    session('working', 'alpha', 'in_progress'),
  ] as const

  expect(buildBoardProjects(sessions)).toEqual([
    {
      name: 'alpha',
      sessions: [sessions[1]],
      repos: ['example/alpha'],
      needsActionCount: 0,
      waitingCount: 0,
      inProgressCount: 1,
      quiet: false,
      status: 'Working',
    },
    {
      name: 'beta',
      sessions: [sessions[0]],
      repos: ['example/beta'],
      needsActionCount: 0,
      waitingCount: 1,
      inProgressCount: 0,
      quiet: false,
      status: 'Waiting',
    },
  ])
})

it('sorts a waiting-only project ahead of quiet projects', () => {
  const sessions = [
    session('waiting', 'zeta', 'waiting'),
    session('quiet', 'alpha', 'done'),
  ] as const

  expect(buildBoardProjects(sessions)).toEqual([
    {
      name: 'zeta',
      sessions: [sessions[0]],
      repos: ['example/zeta'],
      needsActionCount: 0,
      waitingCount: 1,
      inProgressCount: 0,
      quiet: false,
      status: 'Waiting',
    },
    {
      name: 'alpha',
      sessions: [sessions[1]],
      repos: ['example/alpha'],
      needsActionCount: 0,
      waitingCount: 0,
      inProgressCount: 0,
      quiet: true,
      status: 'Idle',
    },
  ])
})

it('groups work types by label and orders Waiting before In progress', () => {
  const sessions = [
    {
      ...session('guide-active', 'sample project', 'in_progress'),
      workType: 'release-guide',
    },
    {
      ...session('guide-waiting', 'sample project', 'waiting'),
      workType: 'release-guide',
    },
    {
      ...session('guide-needs-action', 'sample project', 'needs_action'),
      workType: 'release-guide',
    },
    {
      ...session('other-work', 'sample project', 'needs_action'),
      workType: 'test-review',
    },
    {
      ...session('unconfigured', 'quiet playground', 'done'),
      workType: 'private-flow',
    },
  ] as const

  expect(
    buildWorkTypeNavGroups(sessions, {
      'release-guide': 'Guides',
      'test-review': 'Guides',
      'private-flow': '   ',
    }),
  ).toEqual([
    {
      label: 'Guides',
      sessions: [sessions[2], sessions[3], sessions[1], sessions[0]],
    },
  ])
})

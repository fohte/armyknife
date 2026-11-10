import { expect, it } from 'vitest'

import { buildBoardProjects, sortPmProjects } from '#board-model'
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
    parent: null,
    pullRequest: null,
  }
}

it('sorts projects by action count, activity, then name', () => {
  const projects = buildBoardProjects([
    session('idle-z', 'zeta', 'done'),
    session('working-b', 'beta', 'in_progress'),
    session('action-a', 'alpha', 'needs_action'),
    session('action-a-2', 'alpha', 'needs_action'),
    session('action-c', 'charlie', 'needs_action'),
    session('working-b-2', 'beta', 'in_progress'),
  ])

  expect(projects.map(({ name }) => name)).toEqual([
    'alpha',
    'charlie',
    'beta',
    'zeta',
  ])
})

it('hides only quiet projects when the quiet filter is enabled', () => {
  const projects = buildBoardProjects([
    session('action', 'alpha', 'needs_action'),
    session('working', 'beta', 'in_progress'),
    session('quiet', 'gamma', 'done'),
  ])

  expect(projects.map(({ name, quiet }) => [name, quiet])).toEqual([
    ['alpha', false],
    ['beta', false],
    ['gamma', true],
  ])
})

it('orders PM projects by status before activity and name', () => {
  const projects = buildBoardProjects([
    session('idle', 'alpha', 'done'),
    session('working', 'bravo', 'in_progress'),
    session('review-b', 'charlie', 'needs_action'),
    session('review-a', 'delta', 'needs_action'),
  ])

  expect(
    sortPmProjects(projects).map(({ name, status }) => [name, status]),
  ).toEqual([
    ['charlie', 'Review'],
    ['delta', 'Review'],
    ['bravo', 'Working'],
    ['alpha', 'Idle'],
  ])
})

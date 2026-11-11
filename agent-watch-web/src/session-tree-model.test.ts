import { expect, it } from 'vitest'

import {
  defaultExpandedSessionIds,
  directSessionChildCounts,
  needsActionDescendantCount,
  visibleSessionTreeRows,
} from '#session-tree-model'
import type { SessionTreeNode } from '#types'

const sessions: SessionTreeNode[] = [
  {
    id: 'root',
    parentId: null,
    category: 'done',
    isActive: true,
    kind: null,
    title: 'Root session',
    preview: null,
    elapsedSeconds: 300,
    task: null,
    repo: 'sample/repo',
    decisionCount: 0,
    pullRequest: null,
  },
  {
    id: 'selected',
    parentId: 'root',
    category: 'in_progress',
    isActive: true,
    kind: null,
    title: 'Selected session',
    preview: 'Working on a fixture',
    elapsedSeconds: 40,
    task: null,
    repo: 'sample/repo',
    decisionCount: 0,
    pullRequest: null,
  },
  {
    id: 'action-child',
    parentId: 'selected',
    category: 'needs_action',
    isActive: true,
    kind: 'Question',
    title: 'Action child',
    preview: 'Needs a response',
    elapsedSeconds: 12,
    task: null,
    repo: 'sample/repo',
    decisionCount: 0,
    pullRequest: null,
  },
  {
    id: 'finished-grandchild',
    parentId: 'action-child',
    category: 'done',
    isActive: false,
    kind: null,
    title: 'Finished grandchild',
    preview: null,
    elapsedSeconds: 600,
    task: null,
    repo: 'sample/repo',
    decisionCount: 0,
    pullRequest: null,
  },
  {
    id: 'other-branch',
    parentId: 'root',
    category: 'done',
    isActive: true,
    kind: null,
    title: 'Other branch',
    preview: null,
    elapsedSeconds: 500,
    task: null,
    repo: 'sample/repo',
    decisionCount: 0,
    pullRequest: null,
  },
]

it('opens the selected path and its descendants while counting action descendants', () => {
  const expanded = defaultExpandedSessionIds(sessions, 'selected')

  const actual = () => [
    [...expanded].sort(),
    visibleSessionTreeRows(sessions, expanded).map((row) => ({
      id: row.session.id,
      depth: row.depth,
      expanded: row.isExpanded,
      actionDescendants: row.needsActionDescendantCount,
    })),
    needsActionDescendantCount(sessions, 'selected'),
  ]
  expect(actual()).toEqual([
    ['action-child', 'root', 'selected'],
    [
      { id: 'root', depth: 0, expanded: true, actionDescendants: 1 },
      { id: 'selected', depth: 1, expanded: true, actionDescendants: 1 },
      { id: 'action-child', depth: 2, expanded: true, actionDescendants: 0 },
      {
        id: 'finished-grandchild',
        depth: 3,
        expanded: false,
        actionDescendants: 0,
      },
      { id: 'other-branch', depth: 1, expanded: false, actionDescendants: 0 },
    ],
    1,
  ])
})

it('counts direct session children by parent', () => {
  expect([...directSessionChildCounts(sessions)].sort()).toEqual([
    ['action-child', 1],
    ['root', 2],
    ['selected', 1],
  ])
})

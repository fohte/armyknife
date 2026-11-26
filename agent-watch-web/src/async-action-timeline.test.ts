import { expect, it } from 'vitest'

import type { AsyncAction } from '#async-action-model'
import { shiftActionTimelineIndices } from '#async-action-timeline'

it('rebases every positioned action in the retained session messages', () => {
  const actions: AsyncAction[] = [
    {
      id: 1,
      sessionId: 'fixture-session',
      sessionTitle: 'Fixture session',
      kind: 'merge',
      status: 'merging',
      timelineIndex: 12,
      createdAt: 1,
    },
    {
      id: 2,
      sessionId: 'fixture-session',
      sessionTitle: 'Fixture session',
      kind: 'review_document',
      status: 'reviewDocumentActionRunning',
      timelineIndex: 12,
      createdAt: 2,
    },
    {
      id: 3,
      sessionId: 'fixture-session',
      sessionTitle: 'Fixture session',
      kind: 'command',
      status: 'runningCommand',
      timelineIndex: 12,
      createdAt: 3,
    },
    {
      id: 4,
      sessionId: 'another-session',
      sessionTitle: 'Another session',
      kind: 'review_document',
      status: 'reviewDocumentActionRunning',
      timelineIndex: 12,
      createdAt: 4,
    },
  ]

  expect(
    shiftActionTimelineIndices(actions, {
      sessionId: 'fixture-session',
      from: 10,
      through: 15,
      offset: 3,
    }),
  ).toEqual([
    { ...actions[0], timelineIndex: 15 },
    { ...actions[1], timelineIndex: 15 },
    { ...actions[2], timelineIndex: 15 },
    actions[3],
  ])
})

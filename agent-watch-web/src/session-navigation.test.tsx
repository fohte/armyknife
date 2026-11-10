import { expect, it } from 'vitest'

import { getSessionShortcutAction, moveSelection } from '#session-navigation'
import type { Session } from '#types'

function session(id: string): Session {
  return {
    id,
    category: 'needs_action',
    kind: 'Question',
    title: 'Review a request',
    preview: 'Please choose a retry policy.',
    elapsedSeconds: 10,
    unread: true,
    task: null,
    repo: 'example/service',
    project: 'sample project',
    parent: null,
    pullRequest: null,
  }
}

const sessions = [session('sample-a'), session('sample-b'), session('sample-c')]

it('moves the selected row with j and k and keeps selection in bounds', () => {
  const getSelectionResults = () => [
    moveSelection(sessions, 'sample-a', 'j'),
    moveSelection(sessions, 'sample-c', 'k'),
    moveSelection(sessions, 'sample-a', 'k'),
    moveSelection(sessions, 'sample-c', 'j'),
    moveSelection(sessions, null, 'j'),
    moveSelection([], null, 'j'),
  ]

  expect(getSelectionResults()).toEqual([
    'sample-b',
    'sample-b',
    'sample-a',
    'sample-c',
    'sample-a',
    null,
  ])
})

it('toggles views with shifted B and maps unshifted j and k to row movement', () => {
  const getShortcutResults = () => [
    getSessionShortcutAction('B', true, 'inbox'),
    getSessionShortcutAction('B', true, 'board'),
    getSessionShortcutAction('j', false, 'inbox'),
    getSessionShortcutAction('k', false, 'inbox'),
    getSessionShortcutAction('l', false, 'inbox'),
  ]

  expect(getShortcutResults()).toEqual([
    { type: 'view', view: 'board' },
    { type: 'view', view: 'inbox' },
    { type: 'move', key: 'j' },
    { type: 'move', key: 'k' },
    null,
  ])
})

import { expect, it } from 'vitest'

import {
  getLaterPresetShortcut,
  getSessionShortcutAction,
  moveSelection,
} from '#session-navigation'
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
    ancestorSessionIds: [],
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

it('maps Later and Board shortcuts separately from row movement', () => {
  const getShortcutResults = () => [
    getSessionShortcutAction('B', true, 'inbox'),
    getSessionShortcutAction('B', true, 'board'),
    getSessionShortcutAction('j', false, 'inbox'),
    getSessionShortcutAction('k', false, 'inbox'),
    getSessionShortcutAction('L', false, 'inbox'),
    getSessionShortcutAction('L', true, 'inbox'),
    getSessionShortcutAction('l', true, 'inbox'),
    getSessionShortcutAction('l', false, 'inbox'),
  ]

  expect(getShortcutResults()).toEqual([
    { type: 'view', view: 'board' },
    { type: 'view', view: 'inbox' },
    { type: 'move', key: 'j' },
    { type: 'move', key: 'k' },
    { type: 'later' },
    { type: 'later' },
    { type: 'later' },
    { type: 'later' },
  ])
})

it('maps menu number keys to the five Later presets', () => {
  expect(['1', '2', '3', '4', '5', '6'].map(getLaterPresetShortcut)).toEqual([
    'one_hour',
    'this_evening',
    'tomorrow',
    'next_week',
    'until_it_changes',
    null,
  ])
})

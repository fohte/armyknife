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

it('maps shifted B to Board and unshifted j and k to row movement', () => {
  const getShortcutResults = () => [
    getSessionShortcutAction('B', true),
    getSessionShortcutAction('j', false),
    getSessionShortcutAction('k', false),
    getSessionShortcutAction('L', false),
    getSessionShortcutAction('L', true),
    getSessionShortcutAction('l', true),
    getSessionShortcutAction('l', false),
  ]

  expect(getShortcutResults()).toEqual([
    { type: 'view', view: 'board' },
    { type: 'move', key: 'j' },
    { type: 'move', key: 'k' },
    { type: 'later' },
    { type: 'later' },
    { type: 'later' },
    null,
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

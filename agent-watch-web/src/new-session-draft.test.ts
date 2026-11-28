import { expect, it } from 'vitest'

import {
  decodeNewSessionDraft,
  emptyNewSessionDraftFields,
  encodeNewSessionDraft,
  type NewSessionDraft,
} from '#new-session-draft'

const validDraft: NewSessionDraft = {
  version: 1,
  savedAt: '2026-05-06T12:03:00.000Z',
  directory: { path: '/example/project', source: 'recent' },
  directoryQuery: 'project',
  engine: 'codex',
  selections: {
    claude: { model: null, effort: null },
    codex: { model: 'fixture-model-orbit', effort: 'high' },
  },
  worktree: true,
  initialMessage: 'Continue the setup.',
}

it.each([
  {
    name: 'valid data',
    raw: JSON.stringify(validDraft),
    expected: validDraft,
  },
  { name: 'broken JSON', raw: '{broken', expected: null },
  {
    name: 'another version',
    raw: JSON.stringify({ ...validDraft, version: 2 }),
    expected: null,
  },
  {
    name: 'unknown engine',
    raw: JSON.stringify({ ...validDraft, engine: 'unknown' }),
    expected: null,
  },
  {
    name: 'invalid savedAt',
    raw: JSON.stringify({ ...validDraft, savedAt: 'not-a-date' }),
    expected: null,
  },
])('decodes $name', ({ raw, expected }) => {
  expect(decodeNewSessionDraft(raw)).toEqual(expected)
})

it('encodes the default values as an empty draft', () => {
  expect(
    encodeNewSessionDraft(emptyNewSessionDraftFields(), '2026-05-06T12:03:00Z'),
  ).toEqual('')
})

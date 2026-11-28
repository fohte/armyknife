import { Result } from 'neverthrow'

import {
  isSessionDirectory,
  type SessionDirectory,
  type SessionEngine,
} from '#new-session-api'
import type { NewSessionModelSelection } from '#new-session-selection'

export const NEW_SESSION_DRAFT_STORAGE_KEY = 'agent-watch-web:new-session-draft'

export type NewSessionDraftFields = {
  directory: SessionDirectory | null
  directoryQuery: string
  engine: SessionEngine
  selections: Record<SessionEngine, NewSessionModelSelection>
  worktree: boolean
  initialMessage: string
}

export type NewSessionDraft = NewSessionDraftFields & {
  version: 1
  savedAt: string
}

const defaultSelections: Record<SessionEngine, NewSessionModelSelection> = {
  claude: { model: null, effort: null },
  codex: { model: null, effort: null },
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === 'string'
}

function isModelSelection(value: unknown): value is NewSessionModelSelection {
  return (
    isRecord(value) &&
    isNullableString(value['model']) &&
    isNullableString(value['effort'])
  )
}

function isNewSessionDraft(value: unknown): value is NewSessionDraft {
  return (
    isRecord(value) &&
    value['version'] === 1 &&
    typeof value['savedAt'] === 'string' &&
    !Number.isNaN(Date.parse(value['savedAt'])) &&
    (value['directory'] === null || isSessionDirectory(value['directory'])) &&
    typeof value['directoryQuery'] === 'string' &&
    (value['engine'] === 'claude' || value['engine'] === 'codex') &&
    isRecord(value['selections']) &&
    isModelSelection(value['selections']['claude']) &&
    isModelSelection(value['selections']['codex']) &&
    typeof value['worktree'] === 'boolean' &&
    typeof value['initialMessage'] === 'string'
  )
}

export function decodeNewSessionDraft(raw: string): NewSessionDraft | null {
  return Result.fromThrowable(
    () => JSON.parse(raw) as unknown,
    (error: unknown) => error,
  )().match(
    (value) => (isNewSessionDraft(value) ? value : null),
    () => null,
  )
}

export function isDefaultNewSessionDraft(fields: NewSessionDraftFields) {
  return (
    fields.directory === null &&
    fields.directoryQuery === '' &&
    fields.engine === 'claude' &&
    fields.selections.claude.model === null &&
    fields.selections.claude.effort === null &&
    fields.selections.codex.model === null &&
    fields.selections.codex.effort === null &&
    !fields.worktree &&
    fields.initialMessage === ''
  )
}

export function encodeNewSessionDraft(
  fields: NewSessionDraftFields,
  savedAt: string,
) {
  if (isDefaultNewSessionDraft(fields)) return ''
  return JSON.stringify({
    version: 1,
    savedAt,
    ...fields,
  } satisfies NewSessionDraft)
}

export function emptyNewSessionDraftFields(): NewSessionDraftFields {
  return {
    directory: null,
    directoryQuery: '',
    engine: 'claude',
    selections: {
      claude: { ...defaultSelections.claude },
      codex: { ...defaultSelections.codex },
    },
    worktree: false,
    initialMessage: '',
  }
}

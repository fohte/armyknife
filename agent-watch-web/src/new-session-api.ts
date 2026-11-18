import { requestJson } from '#api-requests'
import { postNewSession } from '#new-session-http'
import type { SkillSuggestion } from '#types'

export type SessionDirectory = {
  path: string
  source: 'recent' | 'repo' | 'path'
}

export type SessionEngine = 'claude' | 'codex'

export type SessionModelOption = {
  id: string
  label: string
  defaultEffort: string | null
  efforts: string[]
}

export type SessionModelOptions = {
  defaultModel: string | null
  defaultEffort: string | null
  models: SessionModelOption[]
}

export type LaunchSessionRequest = {
  directory: string
  engine: SessionEngine
  worktree: boolean
  model?: string
  reasoningEffort?: string
  initialMessage?: string
}

type LaunchSessionResponse = { launchId: string }

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isSessionDirectory(value: unknown): value is SessionDirectory {
  return (
    isRecord(value) &&
    typeof value['path'] === 'string' &&
    (value['source'] === 'recent' ||
      value['source'] === 'repo' ||
      value['source'] === 'path')
  )
}

function isDirectoriesResponse(
  value: unknown,
): value is { directories: SessionDirectory[] } {
  return (
    isRecord(value) &&
    Array.isArray(value['directories']) &&
    value['directories'].every(isSessionDirectory)
  )
}

function isSkillSuggestionsResponse(
  value: unknown,
): value is { skills: SkillSuggestion[] } {
  return (
    isRecord(value) &&
    Array.isArray(value['skills']) &&
    value['skills'].every(
      (skill) =>
        isRecord(skill) &&
        typeof skill['name'] === 'string' &&
        typeof skill['description'] === 'string',
    )
  )
}

function isLaunchSessionResponse(
  value: unknown,
): value is LaunchSessionResponse {
  return isRecord(value) && typeof value['launchId'] === 'string'
}

function isSessionModelOptions(value: unknown): value is SessionModelOptions {
  return (
    isRecord(value) &&
    (typeof value['defaultModel'] === 'string' ||
      value['defaultModel'] === null) &&
    (typeof value['defaultEffort'] === 'string' ||
      value['defaultEffort'] === null) &&
    Array.isArray(value['models']) &&
    value['models'].every(
      (model) =>
        isRecord(model) &&
        typeof model['id'] === 'string' &&
        typeof model['label'] === 'string' &&
        (typeof model['defaultEffort'] === 'string' ||
          model['defaultEffort'] === null) &&
        Array.isArray(model['efforts']) &&
        model['efforts'].every((effort) => typeof effort === 'string'),
    )
  )
}

export function getSessionDirectories(query: string) {
  const search = new URLSearchParams()
  if (query.length > 0) search.set('q', query)
  const suffix = search.size === 0 ? '' : `?${search.toString()}`
  return requestJson(`/api/directories${suffix}`, isDirectoriesResponse).map(
    ({ directories }) => directories,
  )
}

export function getSessionSkillSuggestions(
  engine: SessionEngine,
  directory: string | null,
) {
  const search = new URLSearchParams({ engine })
  if (directory !== null) search.set('directory', directory)
  return requestJson(
    `/api/skill-suggestions?${search.toString()}`,
    isSkillSuggestionsResponse,
  ).map(({ skills }) => skills)
}

export function getSessionModelOptions(engine: SessionEngine) {
  const search = new URLSearchParams({ engine })
  return requestJson(
    `/api/model-options?${search.toString()}`,
    isSessionModelOptions,
  )
}

export function launchSession(request: LaunchSessionRequest) {
  const { initialMessage, model, reasoningEffort, ...fields } = request
  const body = {
    ...fields,
    ...(model === undefined ? {} : { model }),
    ...(reasoningEffort === undefined ? {} : { reasoningEffort }),
    ...(initialMessage === undefined || initialMessage.trim().length === 0
      ? {}
      : { initialMessage }),
  }
  return postNewSession(body, isLaunchSessionResponse)
}

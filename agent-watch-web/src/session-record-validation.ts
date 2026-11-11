import type { PullRequest, SessionTask } from '#types'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

export function isSessionTask(value: unknown): value is SessionTask {
  return (
    isRecord(value) &&
    typeof value['id'] === 'string' &&
    typeof value['number'] === 'number' &&
    typeof value['title'] === 'string' &&
    (value['priority'] === null || typeof value['priority'] === 'number') &&
    (value['isClosed'] === undefined || typeof value['isClosed'] === 'boolean')
  )
}

export function isPullRequest(value: unknown): value is PullRequest {
  return (
    isRecord(value) &&
    typeof value['number'] === 'number' &&
    typeof value['state'] === 'string'
  )
}

import { isPullRequest, isSessionTask } from '#session-record-validation'
import type { SessionTreeNode } from '#types'
import { isVibeMode } from '#vibe-mode'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

export function isSessionTreeResponse(
  value: unknown,
): value is { revision: number; sessions: SessionTreeNode[] } {
  return (
    isRecord(value) &&
    typeof value['revision'] === 'number' &&
    Number.isInteger(value['revision']) &&
    Array.isArray(value['sessions']) &&
    value['sessions'].every(
      (session) =>
        isRecord(session) &&
        typeof session['id'] === 'string' &&
        (session['parentId'] === null ||
          typeof session['parentId'] === 'string') &&
        (session['engine'] === undefined ||
          session['engine'] === 'claude' ||
          session['engine'] === 'codex') &&
        (session['workType'] === undefined ||
          session['workType'] === null ||
          typeof session['workType'] === 'string') &&
        typeof session['category'] === 'string' &&
        typeof session['isActive'] === 'boolean' &&
        (session['kind'] === null || typeof session['kind'] === 'string') &&
        typeof session['title'] === 'string' &&
        (session['preview'] === null ||
          typeof session['preview'] === 'string') &&
        typeof session['elapsedSeconds'] === 'number' &&
        Number.isInteger(session['elapsedSeconds']) &&
        (session['task'] === null || isSessionTask(session['task'])) &&
        typeof session['repo'] === 'string' &&
        isVibeMode(session['vibeMode']) &&
        (session['pullRequest'] === null ||
          isPullRequest(session['pullRequest'])) &&
        (session['pullRequests'] === undefined ||
          (Array.isArray(session['pullRequests']) &&
            session['pullRequests'].every(isPullRequest))),
    )
  )
}

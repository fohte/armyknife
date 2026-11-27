import type { PullRequest, Session } from '#types'

export function getSessionPullRequests(
  session: Pick<Session, 'pullRequest' | 'pullRequests'>,
): readonly PullRequest[] {
  return session.pullRequests !== undefined && session.pullRequests.length > 0
    ? session.pullRequests
    : session.pullRequest === null
      ? []
      : [session.pullRequest]
}

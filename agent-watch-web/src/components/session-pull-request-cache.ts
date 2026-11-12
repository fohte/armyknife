import type { SessionPullRequest } from '#types'

export type CachedPullRequestList = {
  pullRequests: SessionPullRequest[]
  error: string | null
  fetchedAt: number
  lastAttemptAt: number
}

const pullRequestLists = new Map<string, CachedPullRequestList>()
const requestGenerations = new Map<string, number>()

export function getCachedPullRequestList(
  cacheKey: string,
): CachedPullRequestList | undefined {
  return pullRequestLists.get(cacheKey)
}

export function setCachedPullRequestList(
  cacheKey: string,
  cached: CachedPullRequestList,
): void {
  pullRequestLists.set(cacheKey, cached)
}

export function beginPullRequestListRequest(cacheKey: string): number {
  const generation = (requestGenerations.get(cacheKey) ?? 0) + 1
  requestGenerations.set(cacheKey, generation)
  return generation
}

export function isCurrentPullRequestListRequest(
  cacheKey: string,
  generation: number,
): boolean {
  return requestGenerations.get(cacheKey) === generation
}

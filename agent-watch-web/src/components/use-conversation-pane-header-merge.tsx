import { useState } from 'react'

import type { PullRequestMergeFeedbackKind } from '#components/pull-request-merge-feedback'
import {
  getRepresentativePullRequest,
  isPullRequestMergeable,
} from '#components/session-pull-requests'
import { useSessionOperations } from '#session-operation-context'
import type { SessionOperation } from '#session-operations'
import type { PullRequest, Session } from '#types'

type MergeResponse =
  | { result: 'merged' }
  | { result: 'alreadyMerged' }
  | { result: 'enqueued'; message: string }

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

type MergeFeedback = {
  number: number
  owner: string
  repo: string
  kind: PullRequestMergeFeedbackKind
  message: string
}

type PullRequestMergeOperation = Extract<SessionOperation, { type: 'merge' }>

function isMergeResponse(value: unknown): value is MergeResponse {
  if (!isRecord(value)) return false
  const result = value['result']
  return (
    result === 'merged' ||
    result === 'alreadyMerged' ||
    (result === 'enqueued' && typeof value['message'] === 'string')
  )
}

type UseConversationPaneHeaderMergeOptions = {
  session: Session
  pullRequests: readonly PullRequest[]
  setPullRequestListOpen: (open: boolean) => void
}

export function useConversationPaneHeaderMerge({
  session,
  pullRequests,
  setPullRequestListOpen,
}: UseConversationPaneHeaderMergeOptions) {
  const [mergeConfirmationOpen, setMergeConfirmationOpen] = useState(false)
  const [mergeTarget, setMergeTarget] =
    useState<PullRequestMergeOperation | null>(null)
  const [mergeFeedback, setMergeFeedback] = useState<MergeFeedback | null>(null)
  const { pendingOperations, run } = useSessionOperations()
  const sessionRepoParts = session.repo.split('/')
  const mergeIdentity = (target: {
    number: number
    owner?: string
    repo?: string
  }) => ({
    number: target.number,
    owner: target.owner ?? sessionRepoParts[0],
    repo: target.repo ?? sessionRepoParts.at(-1),
  })
  const mergeOperationFor = (
    target: PullRequest,
  ): PullRequestMergeOperation | null => {
    if (!isPullRequestMergeable(target)) return null
    const identity = mergeIdentity(target)
    if (identity.owner === undefined || identity.repo === undefined) return null
    return {
      type: 'merge',
      sessionId: session.id,
      number: identity.number,
      owner: identity.owner,
      repo: identity.repo,
      expectedHeadSha: target.headSha,
    }
  }
  const pullRequest = getRepresentativePullRequest(pullRequests) ?? null
  const headerMergeOperation =
    pullRequest?.isCurrentBranch === true
      ? mergeOperationFor(pullRequest)
      : null
  const isMerging = (target: {
    number: number
    owner?: string
    repo?: string
  }) => {
    const identity = mergeIdentity(target)
    if (identity.owner === undefined || identity.repo === undefined)
      return false
    return pendingOperations.some(
      ({ operation }) =>
        operation.type === 'merge' &&
        operation.sessionId === session.id &&
        operation.number === identity.number &&
        operation.owner === identity.owner &&
        operation.repo === identity.repo,
    )
  }
  const sameMergeIdentity = (
    left: { number: number; owner?: string; repo?: string },
    right: { number: number; owner?: string; repo?: string },
  ) => {
    const leftIdentity = mergeIdentity(left)
    const rightIdentity = mergeIdentity(right)
    return (
      leftIdentity.owner !== undefined &&
      leftIdentity.repo !== undefined &&
      rightIdentity.owner !== undefined &&
      rightIdentity.repo !== undefined &&
      leftIdentity.number === rightIdentity.number &&
      leftIdentity.owner === rightIdentity.owner &&
      leftIdentity.repo === rightIdentity.repo
    )
  }
  const isMergeSuccessful = (target: {
    number: number
    owner?: string
    repo?: string
  }) =>
    mergeFeedback?.kind === 'success' &&
    sameMergeIdentity(mergeFeedback, target)
  const openMergeConfirmation = (target: PullRequest) => {
    const operation = mergeOperationFor(target)
    if (operation === null) return
    setMergeTarget(operation)
    setPullRequestListOpen(false)
    setMergeFeedback(null)
    setMergeConfirmationOpen(true)
  }
  const showHeaderMergeAction =
    headerMergeOperation !== null && !isMergeSuccessful(headerMergeOperation)
  const visibleMergeFeedback =
    mergeFeedback !== null &&
    pullRequests.some((candidate) =>
      sameMergeIdentity(mergeFeedback, candidate),
    )
      ? mergeFeedback
      : null

  const confirmMerge = async () => {
    if (mergeTarget === null || isMerging(mergeTarget)) return
    const identity = {
      number: mergeTarget.number,
      owner: mergeTarget.owner,
      repo: mergeTarget.repo,
    }
    setMergeFeedback(null)
    setMergeConfirmationOpen(false)
    const result = await run(mergeTarget)
    result.match(
      (response) => {
        if (!isMergeResponse(response)) {
          setMergeFeedback({
            ...identity,
            kind: 'error',
            message: 'The server returned an invalid merge response.',
          })
          return
        }
        const message =
          response.result === 'merged'
            ? `PR #${String(mergeTarget.number)} merged.`
            : response.result === 'alreadyMerged'
              ? `PR #${String(mergeTarget.number)} was already merged.`
              : `PR #${String(mergeTarget.number)} was added to the merge queue: ${response.message}`
        setMergeFeedback({
          ...identity,
          kind: response.result === 'enqueued' ? 'queued' : 'success',
          message,
        })
      },
      (error) => {
        setMergeFeedback({
          ...identity,
          kind: 'error',
          message: error.message,
        })
      },
    )
  }

  return {
    confirmMerge,
    headerMergeOperation,
    isMergeSuccessful,
    isMerging,
    mergeConfirmationOpen,
    mergeTarget,
    openMergeConfirmation,
    showHeaderMergeAction,
    closeMergeConfirmation: () => {
      setMergeConfirmationOpen(false)
    },
    visibleMergeFeedback,
  }
}

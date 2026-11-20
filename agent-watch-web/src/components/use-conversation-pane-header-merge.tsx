import { useState } from 'react'

import { useOptionalAsyncActionFeedback } from '#async-action-feedback'
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
  const { pendingOperations, run } = useSessionOperations()
  const actionFeedback = useOptionalAsyncActionFeedback()
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
    return (
      mergeFeedbackFor(target)?.status === 'merging' ||
      pendingOperations.some(
        ({ operation }) =>
          operation.type === 'merge' &&
          operation.sessionId === session.id &&
          operation.number === identity.number &&
          operation.owner === identity.owner &&
          operation.repo === identity.repo,
      )
    )
  }
  const mergeFeedbackFor = (target: {
    number: number
    owner?: string
    repo?: string
  }) => {
    const identity = mergeIdentity(target)
    return actionFeedback?.actions.find(
      (action) =>
        action.kind === 'merge' &&
        action.sessionId === session.id &&
        action.pullRequestNumber === identity.number &&
        action.pullRequestOwner === identity.owner &&
        action.pullRequestRepo === identity.repo,
    )
  }
  const isMergeSuccessful = (target: {
    number: number
    owner?: string
    repo?: string
  }) => {
    const action = mergeFeedbackFor(target)
    return action?.status === 'merged' || action?.status === 'alreadyMerged'
  }
  const mergeActionFor = (target: {
    number: number
    owner?: string
    repo?: string
  }) => mergeFeedbackFor(target)
  const openMergeConfirmation = (target: PullRequest) => {
    const operation = mergeOperationFor(target)
    if (operation === null) return
    setMergeTarget(operation)
    setPullRequestListOpen(false)
    setMergeConfirmationOpen(true)
  }
  const confirmMerge = () => {
    if (mergeTarget === null || isMerging(mergeTarget)) return
    const target = mergeTarget
    setMergeConfirmationOpen(false)
    if (actionFeedback === null) {
      void run(target)
      return
    }
    actionFeedback.start({
      kind: 'merge',
      sessionId: session.id,
      sessionTitle: session.title,
      pullRequestNumber: target.number,
      pullRequestOwner: target.owner,
      pullRequestRepo: target.repo,
      run: async () => {
        const result = await run(target)
        return result.match(
          (response) => {
            if (!isMergeResponse(response)) {
              return {
                status: 'mergeFailed' as const,
                error: 'The server returned an invalid merge response.',
              }
            }
            if (response.result === 'enqueued') {
              return { status: 'queued' as const, message: response.message }
            }
            return { status: response.result }
          },
          (error) => ({ status: 'mergeFailed' as const, error: error.message }),
        )
      },
    })
  }

  return {
    confirmMerge,
    headerMergeOperation,
    isMergeSuccessful,
    isMerging,
    mergeConfirmationOpen,
    mergeTarget,
    openMergeConfirmation,
    showHeaderMergeAction: headerMergeOperation !== null,
    closeMergeConfirmation: () => {
      setMergeConfirmationOpen(false)
    },
    mergeFeedbackFor,
    mergeActionFor,
    retryMerge: (id: number) => actionFeedback?.retry(id),
  }
}

import { useEffect, useState } from 'react'

import type { StartAsyncActionOptions } from '#async-action-feedback'
import { useOptionalAsyncActionFeedback } from '#async-action-feedback'
import { isPullRequestMergeable } from '#components/session-pull-requests'
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
type MergeFeedbackTarget = {
  number: number
  owner?: string
  repo?: string
  headSha?: string | null
  expectedHeadSha?: string
}

function isStaleHeadFailure(message: string) {
  return /head changed(?: since confirmation)?/i.test(message)
}

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
  timelineIndex?: number
  setPullRequestListOpen: (open: boolean) => void
}

export function useConversationPaneHeaderMerge({
  session,
  pullRequests,
  timelineIndex,
  setPullRequestListOpen,
}: UseConversationPaneHeaderMergeOptions) {
  const [mergeConfirmationOpen, setMergeConfirmationOpen] = useState(false)
  const [mergeTarget, setMergeTarget] =
    useState<PullRequestMergeOperation | null>(null)
  const [retryingActionId, setRetryingActionId] = useState<number | null>(null)
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
  const isMergeable = (target: PullRequest) =>
    mergeOperationFor(target) !== null
  const isMerging = (target: MergeFeedbackTarget) => {
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
  const mergeFeedbackFor = (target: MergeFeedbackTarget) => {
    const identity = mergeIdentity(target)
    const targetHeadSha = target.expectedHeadSha ?? target.headSha
    return actionFeedback?.actions.find(
      (action) =>
        action.kind === 'merge' &&
        action.sessionId === session.id &&
        action.pullRequestNumber === identity.number &&
        action.pullRequestOwner === identity.owner &&
        action.pullRequestRepo === identity.repo &&
        (action.status === 'merging' ||
          targetHeadSha == null ||
          action.pullRequestHeadSha === undefined ||
          action.pullRequestHeadSha === targetHeadSha),
    )
  }
  const mergeActionFor = (target: MergeFeedbackTarget) =>
    mergeFeedbackFor(target)
  const mergeStatusFor = (target: PullRequest) => {
    if (target.state === 'Merged') return 'merged' as const
    if (target.state !== 'Open') return null
    const action = mergeFeedbackFor(target)
    if (isMerging(target)) return 'merging' as const
    if (action?.status === 'mergeFailed') return 'mergeFailed' as const
    if (action?.status === 'queued') return 'queued' as const
    if (action?.status === 'merged') return 'merged' as const
    if (action?.status === 'alreadyMerged') return 'alreadyMerged' as const
    return null
  }
  const mergeablePullRequests = pullRequests.filter((candidate) => {
    if (candidate.state !== 'Open' || candidate.headSha == null) return false
    return (
      mergeOperationFor(candidate) !== null ||
      ['merging', 'queued', 'mergeFailed'].includes(
        mergeStatusFor(candidate) ?? '',
      )
    )
  })
  const openMergeConfirmation = (target: PullRequest) => {
    const operation = mergeOperationFor(target)
    if (operation === null) return
    setRetryingActionId(null)
    setMergeTarget(operation)
    setPullRequestListOpen(false)
    setMergeConfirmationOpen(true)
  }

  useEffect(() => {
    if (
      actionFeedback === null ||
      actionFeedback.pendingMergeRetryActionIds.length === 0 ||
      mergeConfirmationOpen
    ) {
      return
    }
    const action = actionFeedback.actions.find(
      (candidate) =>
        actionFeedback.pendingMergeRetryActionIds.includes(candidate.id) &&
        candidate.kind === 'merge' &&
        candidate.sessionId === session.id &&
        candidate.status === 'mergeFailed' &&
        pullRequests.some((pullRequest) => {
          const identity = mergeIdentity(pullRequest)
          return (
            pullRequest.number === candidate.pullRequestNumber &&
            identity.owner === candidate.pullRequestOwner &&
            identity.repo === candidate.pullRequestRepo &&
            mergeOperationFor(pullRequest) !== null
          )
        }),
    )
    if (action === undefined) return
    const latestPullRequest = pullRequests.find((candidate) => {
      const identity = mergeIdentity(candidate)
      return (
        candidate.number === action.pullRequestNumber &&
        identity.owner === action.pullRequestOwner &&
        identity.repo === action.pullRequestRepo
      )
    })
    if (
      latestPullRequest === undefined ||
      mergeOperationFor(latestPullRequest) === null
    ) {
      return
    }
    openMergeConfirmation(latestPullRequest)
    setRetryingActionId(action.id)
    actionFeedback.consumeMergeRetryConfirmation(action.id)
  }, [
    actionFeedback,
    mergeConfirmationOpen,
    mergeOperationFor,
    openMergeConfirmation,
    pullRequests,
    session.id,
  ])

  const mergeActionOptionsFor = (
    target: PullRequestMergeOperation,
  ): StartAsyncActionOptions => {
    let staleHeadFailure = false
    return {
      kind: 'merge',
      sessionId: session.id,
      sessionTitle: session.title,
      pullRequestNumber: target.number,
      pullRequestOwner: target.owner,
      pullRequestRepo: target.repo,
      pullRequestHeadSha: target.expectedHeadSha,
      ...(timelineIndex === undefined ? {} : { timelineIndex }),
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
          (error) => {
            staleHeadFailure = isStaleHeadFailure(error.message)
            return { status: 'mergeFailed' as const, error: error.message }
          },
        )
      },
      onRetry: (retry, actionId) => {
        if (!staleHeadFailure) {
          retry()
          return
        }
        actionFeedback?.requestMergeRetryConfirmation(actionId)
      },
    }
  }

  const confirmMerge = () => {
    if (mergeTarget === null || isMerging(mergeTarget)) return
    const target = mergeTarget
    setMergeConfirmationOpen(false)
    if (actionFeedback === null) {
      void run(target)
      return
    }
    const options = mergeActionOptionsFor(target)
    if (retryingActionId === null) actionFeedback.start(options)
    else actionFeedback.retryWith(retryingActionId, options)
    if (retryingActionId !== null) {
      actionFeedback.finishMergeRetryConfirmation(retryingActionId)
    }
    setRetryingActionId(null)
  }

  return {
    confirmMerge,
    isMerging,
    mergeConfirmationOpen,
    mergeTarget,
    openMergeConfirmation,
    closeMergeConfirmation: () => {
      if (retryingActionId !== null) {
        actionFeedback?.finishMergeRetryConfirmation(retryingActionId)
      }
      setMergeConfirmationOpen(false)
      setRetryingActionId(null)
    },
    mergeFeedbackFor,
    mergeActionFor,
    mergeStatusFor,
    isMergeable,
    mergeablePullRequests,
    retryMerge: (id: number) => actionFeedback?.retry(id),
  }
}

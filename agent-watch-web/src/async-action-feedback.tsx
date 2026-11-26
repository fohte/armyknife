import { err, ok, type Result } from 'neverthrow'
import type { ReactNode, RefObject } from 'react'
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from 'react'

import {
  type AsyncAction,
  type AsyncActionOutcome,
  isAsyncActionRunningStatus,
  runningStatusFor,
  type StartAsyncActionOptions,
} from '#async-action-model'
import {
  shiftMergeTimelineIndices as shiftMergeActionTimelineIndices,
  type ShiftMergeTimelineIndicesOptions,
} from '#async-action-timeline'
import { AsyncActionFeedbackHost } from '#components/async-action-feedback-host'

export type {
  AsyncAction,
  AsyncActionOutcome,
  AsyncActionStatus,
  StartAsyncActionOptions,
} from '#async-action-model'
export {
  formatAsyncActionTime,
  isAsyncActionFailureStatus,
  isAsyncActionRunningStatus,
} from '#async-action-model'

type AsyncActionRetryUpdate = Pick<
  StartAsyncActionOptions,
  'run' | 'onRetry' | 'pullRequestHeadSha'
>
type AsyncActionFeedbackValue = {
  actions: readonly AsyncAction[]
  toastIds: readonly number[]
  toastRef: RefObject<HTMLDivElement | null>
  activityOpen: boolean
  activityPosition: { left: number; bottom: number } | null
  toastBottomOffset: number | null
  setToastBottomOffset: (offset: number | null) => void
  pendingMergeRetryActionIds: readonly number[]
  pendingCommandOutputActionIds: readonly number[]
  activeMergeRetryConfirmationActionId: number | null
  start: (options: StartAsyncActionOptions) => number
  retry: (id: number) => void
  retryWith: (id: number, update: AsyncActionRetryUpdate) => void
  shiftMergeTimelineIndices: (options: ShiftMergeTimelineIndicesOptions) => void
  requestMergeRetryConfirmation: (id: number) => void
  consumeMergeRetryConfirmation: (id: number) => void
  finishMergeRetryConfirmation: (id: number) => void
  requestCommandOutput: (id: number) => void
  consumeCommandOutput: (id: number) => void
  dismissToast: (id: number) => void
  clearFinished: () => void
  openActivity: (actionId?: number, anchor?: HTMLElement) => void
  closeActivity: () => void
  closeWithWorktree: (id: number) => void
  openInTerminal: (id: number) => void
}

const AsyncActionFeedbackContext =
  createContext<AsyncActionFeedbackValue | null>(null)

const AUTO_DISMISS_MS = 5_000

export function AsyncActionFeedbackProvider({
  children,
}: {
  children: ReactNode
}) {
  const [actions, setActions] = useState<AsyncAction[]>([])
  const [toastIds, setToastIds] = useState<number[]>([])
  const [activityOpen, setActivityOpen] = useState(false)
  const [activityPosition, setActivityPosition] = useState<{
    left: number
    bottom: number
  } | null>(null)
  const [toastBottomOffset, setToastBottomOffset] = useState<number | null>(
    null,
  )
  const [pendingMergeRetryActionIds, setPendingMergeRetryActionIds] = useState<
    number[]
  >([])
  const [pendingCommandOutputActionIds, setPendingCommandOutputActionIds] =
    useState<number[]>([])
  const [
    activeMergeRetryConfirmationActionId,
    setActiveMergeRetryConfirmationActionId,
  ] = useState<number | null>(null)
  const [selectedActionId, setSelectedActionId] = useState<number | null>(null)
  const toastRef = useRef<HTMLDivElement>(null)
  const nextId = useRef(0)
  const runs = useRef(new Map<number, () => Promise<AsyncActionOutcome>>())
  const retryHandlers = useRef(
    new Map<number, (retry: () => void, actionId: number) => void>(),
  )
  const closeWithWorktreeHandlers = useRef(new Map<number, () => void>())
  const openInTerminalHandlers = useRef(new Map<number, () => void>())
  const timers = useRef(new Map<number, ReturnType<typeof setTimeout>>())

  const dismissToast = useCallback((id: number) => {
    const timer = timers.current.get(id)
    if (timer !== undefined) clearTimeout(timer)
    timers.current.delete(id)
    setToastIds((current) => current.filter((toastId) => toastId !== id))
  }, [])

  const runAction = useCallback(
    async (id: number, run: () => Promise<AsyncActionOutcome>) => {
      const timer = timers.current.get(id)
      if (timer !== undefined) clearTimeout(timer)
      timers.current.delete(id)
      setToastIds((current) =>
        current.includes(id) ? current : [id, ...current],
      )
      setActions((current) =>
        current.map((action) => {
          if (action.id !== id) return action
          const next: AsyncAction = {
            ...action,
            status: runningStatusFor(action.kind),
          }
          delete next.error
          delete next.retryable
          delete next.message
          delete next.checks
          delete next.completedAt
          return next
        }),
      )
      const outcome = await run()
      const completedAt = Date.now()
      setActions((current) =>
        current.map((action) => {
          if (action.id !== id) return action
          const next: AsyncAction = {
            ...action,
            status: outcome.status,
            completedAt,
          }
          if ('message' in outcome) next.message = outcome.message
          if ('error' in outcome) next.error = outcome.error
          else delete next.error
          if ('retryable' in outcome) {
            next.retryable = outcome.retryable
          } else {
            delete next.retryable
          }
          if ('checks' in outcome) {
            next.checks = outcome.checks
          } else {
            delete next.checks
          }
          if ('exitCode' in outcome) {
            next.exitCode = outcome.exitCode
          } else {
            delete next.exitCode
          }
          if ('commandOutputIndex' in outcome) {
            next.commandOutputIndex = outcome.commandOutputIndex
          } else {
            delete next.commandOutputIndex
          }
          return next
        }),
      )
      if (
        outcome.status === 'merged' ||
        outcome.status === 'alreadyMerged' ||
        outcome.status === 'queued' ||
        outcome.status === 'closed' ||
        outcome.status === 'rewound' ||
        outcome.status === 'ranCommand' ||
        outcome.status === 'reviewDocumentActionSucceeded'
      ) {
        const timer = setTimeout(() => {
          dismissToast(id)
        }, AUTO_DISMISS_MS)
        timers.current.set(id, timer)
      }
    },
    [dismissToast],
  )

  const start = useCallback(
    (options: StartAsyncActionOptions) => {
      const id = nextId.current++
      runs.current.set(id, options.run)
      if (options.onRetry !== undefined) {
        retryHandlers.current.set(id, options.onRetry)
      }
      if (options.onCloseKeepWorktree !== undefined) {
        closeWithWorktreeHandlers.current.set(id, options.onCloseKeepWorktree)
      }
      if (options.onOpenInTerminal !== undefined) {
        openInTerminalHandlers.current.set(id, options.onOpenInTerminal)
      }
      setActions((current) => [
        {
          id,
          kind: options.kind,
          sessionId: options.sessionId,
          sessionTitle: options.sessionTitle,
          status: runningStatusFor(options.kind),
          ...(options.command === undefined
            ? {}
            : { command: options.command }),
          ...(options.reviewDocumentAction === undefined
            ? {}
            : { reviewDocumentAction: options.reviewDocumentAction }),
          ...(options.pullRequestNumber === undefined
            ? {}
            : { pullRequestNumber: options.pullRequestNumber }),
          ...(options.pullRequestOwner === undefined
            ? {}
            : { pullRequestOwner: options.pullRequestOwner }),
          ...(options.pullRequestRepo === undefined
            ? {}
            : { pullRequestRepo: options.pullRequestRepo }),
          ...(options.pullRequestHeadSha === undefined
            ? {}
            : { pullRequestHeadSha: options.pullRequestHeadSha }),
          ...(options.timelineIndex === undefined
            ? {}
            : { timelineIndex: options.timelineIndex }),
          createdAt: Date.now(),
        },
        ...current,
      ])
      setToastIds((current) => [id, ...current])
      void runAction(id, options.run)
      return id
    },
    [runAction],
  )

  const retry = useCallback(
    (id: number) => {
      const run = runs.current.get(id)
      if (run === undefined) return
      const retryAction = () => {
        void runAction(id, run)
      }
      const handler = retryHandlers.current.get(id)
      if (handler === undefined) retryAction()
      else handler(retryAction, id)
    },
    [runAction],
  )

  const retryWith = useCallback(
    (id: number, update: AsyncActionRetryUpdate) => {
      if (!runs.current.has(id)) return
      runs.current.set(id, update.run)
      if (update.onRetry === undefined) retryHandlers.current.delete(id)
      else retryHandlers.current.set(id, update.onRetry)
      setActions((current) =>
        current.map((action) =>
          action.id !== id || update.pullRequestHeadSha === undefined
            ? action
            : { ...action, pullRequestHeadSha: update.pullRequestHeadSha },
        ),
      )
      void runAction(id, update.run)
    },
    [runAction],
  )

  const shiftMergeTimelineIndices = useCallback(
    (options: ShiftMergeTimelineIndicesOptions) => {
      setActions((current) => shiftMergeActionTimelineIndices(current, options))
    },
    [],
  )

  const requestMergeRetryConfirmation = useCallback((id: number) => {
    setPendingMergeRetryActionIds((current) =>
      current.includes(id) ? current : [...current, id],
    )
  }, [])

  const consumeMergeRetryConfirmation = useCallback((id: number) => {
    setPendingMergeRetryActionIds((current) =>
      current.filter((actionId) => actionId !== id),
    )
    setActiveMergeRetryConfirmationActionId(id)
  }, [])

  const finishMergeRetryConfirmation = useCallback((id: number) => {
    setActiveMergeRetryConfirmationActionId((current) =>
      current === id ? null : current,
    )
  }, [])

  const requestCommandOutput = useCallback(
    (id: number) => {
      setPendingCommandOutputActionIds((current) =>
        current.includes(id) ? current : [...current, id],
      )
      dismissToast(id)
    },
    [dismissToast],
  )

  const consumeCommandOutput = useCallback((id: number) => {
    setPendingCommandOutputActionIds((current) =>
      current.filter((actionId) => actionId !== id),
    )
  }, [])

  const clearFinished = useCallback(() => {
    const finished = new Set(
      actions
        .filter((action) => !isAsyncActionRunningStatus(action.status))
        .map((action) => action.id),
    )
    setActions((current) =>
      current.filter((action) => !finished.has(action.id)),
    )
    setToastIds((current) => current.filter((id) => !finished.has(id)))
    setPendingMergeRetryActionIds((current) =>
      current.filter((id) => !finished.has(id)),
    )
    setPendingCommandOutputActionIds((current) =>
      current.filter((id) => !finished.has(id)),
    )
    for (const id of finished) {
      runs.current.delete(id)
      const timer = timers.current.get(id)
      if (timer !== undefined) clearTimeout(timer)
      timers.current.delete(id)
      retryHandlers.current.delete(id)
      closeWithWorktreeHandlers.current.delete(id)
      openInTerminalHandlers.current.delete(id)
    }
  }, [actions])

  const openActivity = useCallback(
    (actionId?: number, anchor?: HTMLElement) => {
      setSelectedActionId(actionId ?? null)
      if (anchor === undefined) {
        setActivityPosition(null)
      } else {
        const rect = anchor.getBoundingClientRect()
        setActivityPosition({
          left: Math.round(rect.right + 16),
          bottom: Math.round(window.innerHeight - rect.bottom),
        })
      }
      setActivityOpen(true)
    },
    [],
  )

  const closeActivity = useCallback(() => {
    setActivityOpen(false)
    setSelectedActionId(null)
    setActivityPosition(null)
  }, [])

  const closeWithWorktree = useCallback((id: number) => {
    closeWithWorktreeHandlers.current.get(id)?.()
  }, [])

  const openInTerminal = useCallback((id: number) => {
    openInTerminalHandlers.current.get(id)?.()
  }, [])

  useEffect(
    () => () => {
      timers.current.forEach(clearTimeout)
      timers.current.clear()
    },
    [],
  )

  return (
    <AsyncActionFeedbackContext.Provider
      value={{
        actions,
        toastIds,
        toastRef,
        activityOpen,
        activityPosition,
        toastBottomOffset,
        setToastBottomOffset,
        pendingMergeRetryActionIds,
        pendingCommandOutputActionIds,
        activeMergeRetryConfirmationActionId,
        start,
        retry,
        retryWith,
        shiftMergeTimelineIndices,
        requestMergeRetryConfirmation,
        consumeMergeRetryConfirmation,
        finishMergeRetryConfirmation,
        requestCommandOutput,
        consumeCommandOutput,
        dismissToast,
        clearFinished,
        openActivity,
        closeActivity,
        closeWithWorktree,
        openInTerminal,
      }}
    >
      {children}
      <AsyncActionFeedbackHost
        actions={actions}
        toastIds={toastIds}
        toastRef={toastRef}
        activityOpen={activityOpen}
        activityPosition={activityPosition}
        toastBottomOffset={toastBottomOffset}
        selectedActionId={selectedActionId}
        retry={retry}
        dismissToast={dismissToast}
        clearFinished={clearFinished}
        openActivity={openActivity}
        closeActivity={closeActivity}
        closeWithWorktree={closeWithWorktree}
        openInTerminal={openInTerminal}
        showCommandOutput={requestCommandOutput}
      />
    </AsyncActionFeedbackContext.Provider>
  )
}

function useAsyncActionFeedback(): Result<AsyncActionFeedbackValue, Error> {
  const context = useContext(AsyncActionFeedbackContext)
  return context === null
    ? err(
        new Error(
          'useAsyncActionFeedback must be used inside AsyncActionFeedbackProvider',
        ),
      )
    : ok(context)
}

export function useAsyncActionStarter(onProviderError: (error: Error) => void) {
  const feedback = useAsyncActionFeedback()
  return feedback.match(
    ({ start }) => start,
    (error) => (options: StartAsyncActionOptions) => {
      onProviderError(error)
      void options.run()
      return -1
    },
  )
}

export function useOptionalAsyncActionFeedback() {
  return useContext(AsyncActionFeedbackContext)
}

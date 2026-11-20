import { err, ok, type Result } from 'neverthrow'
import type { ReactNode } from 'react'
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from 'react'

import { AsyncActionFeedbackHost } from '#components/async-action-feedback-host'

export type AsyncActionStatus =
  | 'merging'
  | 'merged'
  | 'alreadyMerged'
  | 'queued'
  | 'mergeFailed'
  | 'closing'
  | 'closed'
  | 'closeFailed'

export type AsyncAction = {
  id: number
  sessionId: string
  sessionTitle: string
  kind: 'merge' | 'close'
  status: AsyncActionStatus
  pullRequestNumber?: number
  pullRequestOwner?: string
  pullRequestRepo?: string
  pullRequestHeadSha?: string
  message?: string
  error?: string
  createdAt: number
}

export type AsyncActionOutcome =
  | { status: 'merged' | 'alreadyMerged' | 'queued'; message?: string }
  | { status: 'closed' }
  | { status: 'mergeFailed' | 'closeFailed'; error: string }

export type StartAsyncActionOptions = {
  kind: 'merge' | 'close'
  sessionId: string
  sessionTitle: string
  pullRequestNumber?: number
  pullRequestOwner?: string
  pullRequestRepo?: string
  pullRequestHeadSha?: string
  run: () => Promise<AsyncActionOutcome>
  onRetry?: (retry: () => void, actionId: number) => void
}

type AsyncActionRetryUpdate = Pick<
  StartAsyncActionOptions,
  'run' | 'onRetry' | 'pullRequestHeadSha'
>

type AsyncActionFeedbackValue = {
  actions: readonly AsyncAction[]
  toastIds: readonly number[]
  activityOpen: boolean
  pendingMergeRetryActionIds: readonly number[]
  start: (options: StartAsyncActionOptions) => number
  retry: (id: number) => void
  retryWith: (id: number, update: AsyncActionRetryUpdate) => void
  requestMergeRetryConfirmation: (id: number) => void
  consumeMergeRetryConfirmation: (id: number) => void
  dismissToast: (id: number) => void
  clearFinished: () => void
  openActivity: (actionId?: number) => void
  closeActivity: () => void
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
  const [pendingMergeRetryActionIds, setPendingMergeRetryActionIds] = useState<
    number[]
  >([])
  const [selectedActionId, setSelectedActionId] = useState<number | null>(null)
  const nextId = useRef(0)
  const runs = useRef(new Map<number, () => Promise<AsyncActionOutcome>>())
  const retryHandlers = useRef(
    new Map<number, (retry: () => void, actionId: number) => void>(),
  )
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
            status: action.kind === 'merge' ? 'merging' : 'closing',
          }
          delete next.error
          delete next.message
          return next
        }),
      )
      const outcome = await run()
      setActions((current) =>
        current.map((action) => {
          if (action.id !== id) return action
          const next: AsyncAction = { ...action, status: outcome.status }
          if ('message' in outcome) next.message = outcome.message
          if ('error' in outcome) next.error = outcome.error
          else delete next.error
          return next
        }),
      )
      if (
        outcome.status === 'merged' ||
        outcome.status === 'alreadyMerged' ||
        outcome.status === 'queued' ||
        outcome.status === 'closed'
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
      setActions((current) => [
        {
          id,
          kind: options.kind,
          sessionId: options.sessionId,
          sessionTitle: options.sessionTitle,
          status: options.kind === 'merge' ? 'merging' : 'closing',
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

  const requestMergeRetryConfirmation = useCallback((id: number) => {
    setPendingMergeRetryActionIds((current) =>
      current.includes(id) ? current : [...current, id],
    )
  }, [])

  const consumeMergeRetryConfirmation = useCallback((id: number) => {
    setPendingMergeRetryActionIds((current) =>
      current.filter((actionId) => actionId !== id),
    )
  }, [])

  const clearFinished = useCallback(() => {
    const finished = new Set(
      actions
        .filter(
          (action) =>
            action.status !== 'merging' && action.status !== 'closing',
        )
        .map((action) => action.id),
    )
    setActions((current) =>
      current.filter((action) => !finished.has(action.id)),
    )
    setToastIds((current) => current.filter((id) => !finished.has(id)))
    setPendingMergeRetryActionIds((current) =>
      current.filter((id) => !finished.has(id)),
    )
    for (const id of finished) {
      runs.current.delete(id)
      const timer = timers.current.get(id)
      if (timer !== undefined) clearTimeout(timer)
      timers.current.delete(id)
      retryHandlers.current.delete(id)
    }
  }, [actions])

  const openActivity = useCallback((actionId?: number) => {
    setSelectedActionId(actionId ?? null)
    setActivityOpen(true)
  }, [])

  const closeActivity = useCallback(() => {
    setActivityOpen(false)
    setSelectedActionId(null)
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
        activityOpen,
        pendingMergeRetryActionIds,
        start,
        retry,
        retryWith,
        requestMergeRetryConfirmation,
        consumeMergeRetryConfirmation,
        dismissToast,
        clearFinished,
        openActivity,
        closeActivity,
      }}
    >
      {children}
      <AsyncActionFeedbackHost
        actions={actions}
        toastIds={toastIds}
        activityOpen={activityOpen}
        selectedActionId={selectedActionId}
        retry={retry}
        dismissToast={dismissToast}
        clearFinished={clearFinished}
        openActivity={openActivity}
        closeActivity={closeActivity}
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

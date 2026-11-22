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

import { AsyncActionFeedbackHost } from '#components/async-action-feedback-host'
import type { CloseWorktreeChecks } from '#session-operation-types'

export type AsyncActionStatus =
  | 'runningCommand'
  | 'ranCommand'
  | 'commandFailed'
  | 'commandTimedOut'
  | 'merging'
  | 'merged'
  | 'alreadyMerged'
  | 'queued'
  | 'mergeFailed'
  | 'closing'
  | 'closed'
  | 'closeFailed'

export function isAsyncActionFailureStatus(status: AsyncActionStatus) {
  return (
    status === 'mergeFailed' ||
    status === 'closeFailed' ||
    status === 'commandFailed' ||
    status === 'commandTimedOut'
  )
}

export function isAsyncActionRunningStatus(status: AsyncActionStatus) {
  return (
    status === 'merging' || status === 'closing' || status === 'runningCommand'
  )
}

export function formatAsyncActionTime(timestamp: number) {
  const date = new Date(timestamp)
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`
}

export type AsyncAction = {
  id: number
  sessionId: string
  sessionTitle: string
  kind: 'merge' | 'close' | 'command'
  status: AsyncActionStatus
  command?: string
  commandOutputIndex?: number
  exitCode?: number
  pullRequestNumber?: number
  pullRequestOwner?: string
  pullRequestRepo?: string
  pullRequestHeadSha?: string
  message?: string
  error?: string
  checks?: CloseWorktreeChecks
  createdAt: number
}

export type AsyncActionOutcome =
  | {
      status: 'ranCommand' | 'commandTimedOut'
      commandOutputIndex?: number
    }
  | {
      status: 'commandFailed'
      exitCode?: number
      error?: string
      commandOutputIndex?: number
    }
  | { status: 'merged' | 'alreadyMerged' | 'queued'; message?: string }
  | { status: 'closed' }
  | {
      status: 'mergeFailed' | 'closeFailed'
      error: string
      checks?: CloseWorktreeChecks
    }

export type StartAsyncActionOptions = {
  kind: 'merge' | 'close' | 'command'
  sessionId: string
  sessionTitle: string
  command?: string
  pullRequestNumber?: number
  pullRequestOwner?: string
  pullRequestRepo?: string
  pullRequestHeadSha?: string
  run: () => Promise<AsyncActionOutcome>
  onRetry?: (retry: () => void, actionId: number) => void
  onCloseKeepWorktree?: () => void
  onOpenInTerminal?: () => void
}

type AsyncActionRetryUpdate = Pick<
  StartAsyncActionOptions,
  'run' | 'onRetry' | 'pullRequestHeadSha'
>

function runningStatusFor(kind: AsyncAction['kind']): AsyncActionStatus {
  switch (kind) {
    case 'merge':
      return 'merging'
    case 'close':
      return 'closing'
    case 'command':
      return 'runningCommand'
  }
}

type AsyncActionFeedbackValue = {
  actions: readonly AsyncAction[]
  toastIds: readonly number[]
  toastRef: RefObject<HTMLDivElement | null>
  activityOpen: boolean
  toastBottomOffset: number | null
  setToastBottomOffset: (offset: number | null) => void
  pendingMergeRetryActionIds: readonly number[]
  pendingCommandOutputActionIds: readonly number[]
  activeMergeRetryConfirmationActionId: number | null
  start: (options: StartAsyncActionOptions) => number
  retry: (id: number) => void
  retryWith: (id: number, update: AsyncActionRetryUpdate) => void
  requestMergeRetryConfirmation: (id: number) => void
  consumeMergeRetryConfirmation: (id: number) => void
  finishMergeRetryConfirmation: (id: number) => void
  requestCommandOutput: (id: number) => void
  consumeCommandOutput: (id: number) => void
  dismissToast: (id: number) => void
  clearFinished: () => void
  openActivity: (actionId?: number) => void
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
          delete next.message
          delete next.checks
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
        outcome.status === 'ranCommand'
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

  const openActivity = useCallback((actionId?: number) => {
    setSelectedActionId(actionId ?? null)
    setActivityOpen(true)
  }, [])

  const closeActivity = useCallback(() => {
    setActivityOpen(false)
    setSelectedActionId(null)
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
        toastBottomOffset,
        setToastBottomOffset,
        pendingMergeRetryActionIds,
        pendingCommandOutputActionIds,
        activeMergeRetryConfirmationActionId,
        start,
        retry,
        retryWith,
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

import { createContext, type RefObject } from 'react'

import type { AsyncAction, StartAsyncActionOptions } from '#async-action-model'
import type { ShiftActionTimelineIndicesOptions } from '#async-action-timeline'

export type AsyncActionRetryUpdate = Pick<
  StartAsyncActionOptions,
  'run' | 'onRetry' | 'pullRequestHeadSha'
>

export type AsyncActionFeedbackValue = {
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
  shiftActionTimelineIndices: (
    options: ShiftActionTimelineIndicesOptions,
  ) => void
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
  discardChanges: (id: number) => void
  reviewDiscardAgain: (id: number) => void
  openInTerminal: (id: number) => void
  openSession: (id: number) => void
}

export const AsyncActionFeedbackContext =
  createContext<AsyncActionFeedbackValue | null>(null)

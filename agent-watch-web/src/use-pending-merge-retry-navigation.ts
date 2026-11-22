import { useEffect } from 'react'

import { useOptionalAsyncActionFeedback } from '#async-action-feedback'
import type { Session } from '#types'

type PendingMergeRetryNavigationOptions = {
  isMobile: boolean
  view: 'inbox' | 'board'
  selectedId: string | null
  conversationOpen: boolean
  newSessionMode: 'form' | 'palette' | null
  sessions: readonly Session[]
  openSessionInInbox: (id: string) => void
}

type ConversationVisibility = Pick<
  PendingMergeRetryNavigationOptions,
  'isMobile' | 'view' | 'selectedId' | 'conversationOpen' | 'newSessionMode'
>

function isConversationVisibleFor(
  sessionId: string,
  {
    isMobile,
    view,
    selectedId,
    conversationOpen,
    newSessionMode,
  }: ConversationVisibility,
) {
  return (
    view === 'inbox' &&
    selectedId === sessionId &&
    newSessionMode === null &&
    (!isMobile || conversationOpen)
  )
}

export function usePendingMergeRetryNavigation({
  isMobile,
  view,
  selectedId,
  conversationOpen,
  newSessionMode,
  sessions,
  openSessionInInbox,
}: PendingMergeRetryNavigationOptions) {
  const actionFeedback = useOptionalAsyncActionFeedback()
  const pendingAction = actionFeedback?.actions.find(
    (action) =>
      actionFeedback.pendingMergeRetryActionIds.includes(action.id) &&
      action.kind === 'merge' &&
      action.status === 'mergeFailed',
  )
  const pendingCommandOutputAction = actionFeedback?.actions.find(
    (action) =>
      actionFeedback.pendingCommandOutputActionIds.includes(action.id) &&
      action.kind === 'command',
  )
  const activeRetryActionId =
    actionFeedback?.activeMergeRetryConfirmationActionId ?? null
  const activeRetryAction = actionFeedback?.actions.find(
    (action) => action.id === activeRetryActionId,
  )
  const visibility = {
    isMobile,
    view,
    selectedId,
    conversationOpen,
    newSessionMode,
  }

  useEffect(() => {
    if (
      activeRetryAction !== undefined &&
      (activeRetryAction.status === 'mergeFailed' ||
        activeRetryAction.status === 'merging') &&
      isConversationVisibleFor(activeRetryAction.sessionId, visibility)
    ) {
      return
    }
    if (activeRetryActionId !== null) {
      actionFeedback?.finishMergeRetryConfirmation(activeRetryActionId)
    }

    if (
      pendingAction === undefined ||
      !sessions.some((session) => session.id === pendingAction.sessionId) ||
      isConversationVisibleFor(pendingAction.sessionId, visibility)
    ) {
      return
    }

    actionFeedback?.closeActivity()
    openSessionInInbox(pendingAction.sessionId)
  }, [
    actionFeedback,
    activeRetryAction,
    activeRetryActionId,
    openSessionInInbox,
    pendingAction,
    sessions,
    visibility,
  ])

  useEffect(() => {
    if (
      pendingCommandOutputAction === undefined ||
      !sessions.some(
        (session) => session.id === pendingCommandOutputAction.sessionId,
      ) ||
      isConversationVisibleFor(pendingCommandOutputAction.sessionId, visibility)
    ) {
      return
    }

    actionFeedback?.closeActivity()
    openSessionInInbox(pendingCommandOutputAction.sessionId)
  }, [
    actionFeedback,
    openSessionInInbox,
    pendingCommandOutputAction,
    sessions,
    visibility,
  ])
}

import { useEffect, useRef } from 'react'

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
  const activeRetryActionId = useRef<number | null>(null)
  const pendingAction = actionFeedback?.actions.find(
    (action) =>
      actionFeedback.pendingMergeRetryActionIds.includes(action.id) &&
      action.kind === 'merge' &&
      action.status === 'mergeFailed',
  )

  useEffect(() => {
    const activeRetryAction = actionFeedback?.actions.find(
      (action) => action.id === activeRetryActionId.current,
    )
    if (
      activeRetryAction !== undefined &&
      (activeRetryAction.status === 'mergeFailed' ||
        activeRetryAction.status === 'merging') &&
      view === 'inbox' &&
      selectedId === activeRetryAction.sessionId &&
      newSessionMode === null &&
      (!isMobile || conversationOpen)
    ) {
      return
    }
    activeRetryActionId.current = null

    if (
      pendingAction === undefined ||
      !sessions.some((session) => session.id === pendingAction.sessionId)
    ) {
      return
    }
    const conversationVisible =
      view === 'inbox' &&
      selectedId === pendingAction.sessionId &&
      newSessionMode === null &&
      (!isMobile || conversationOpen)
    if (conversationVisible) return

    activeRetryActionId.current = pendingAction.id
    actionFeedback?.closeActivity()
    openSessionInInbox(pendingAction.sessionId)
  }, [
    actionFeedback,
    conversationOpen,
    isMobile,
    newSessionMode,
    openSessionInInbox,
    pendingAction,
    selectedId,
    sessions,
    view,
  ])
}

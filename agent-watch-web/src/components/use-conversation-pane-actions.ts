import { type Dispatch, type SetStateAction, useCallback } from 'react'

import { isRewindResponse } from '#components/conversation-pane-response'
import {
  requestAction,
  requestActionJson,
} from '#components/conversation-requests'
import type {
  PendingPermissionActions,
  PermissionRequestIdentity,
} from '#components/permission-card'
import { writeComposerDraftForSession } from '#components/use-composer-draft'
import type { RewindMode } from '#types'

type UseConversationPaneActionsOptions = {
  sessionId: string
  refresh: () => void
  setError: Dispatch<SetStateAction<string | null>>
  setPendingPermissionActions: Dispatch<
    SetStateAction<PendingPermissionActions>
  >
  onSelectSession: (id: string) => void
}

export function useConversationPaneActions({
  sessionId,
  refresh,
  setError,
  setPendingPermissionActions,
  onSelectSession,
}: UseConversationPaneActionsOptions) {
  const onAction = useCallback(
    (
      action: string,
      payload?: Record<string, unknown>,
      permissionIdentity?: PermissionRequestIdentity,
    ) => {
      const permissionAction =
        action === 'permission/approve'
          ? 'approve'
          : action === 'permission/reject'
            ? 'reject'
            : action.startsWith('permission-denials/') &&
                action.endsWith('/retry')
              ? 'retry'
              : null
      if (permissionAction !== null && permissionIdentity !== undefined) {
        setPendingPermissionActions((pending) => ({
          ...pending,
          [permissionIdentity.requestId]: {
            action: permissionAction,
            status: 'sending',
            ...permissionIdentity,
          },
        }))
      }
      return requestAction(sessionId, action, payload).match(
        () => {
          if (permissionAction !== null && permissionIdentity !== undefined) {
            setPendingPermissionActions((pending) => ({
              ...pending,
              [permissionIdentity.requestId]: {
                action: permissionAction,
                status: 'sent',
                ...permissionIdentity,
              },
            }))
          }
          setError(null)
          refresh()
          return true
        },
        (requestError) => {
          if (permissionAction !== null && permissionIdentity !== undefined) {
            setPendingPermissionActions((pending) =>
              Object.fromEntries(
                Object.entries(pending).filter(
                  ([requestId]) => requestId !== permissionIdentity.requestId,
                ),
              ),
            )
          }
          setError(requestError.message)
          return false
        },
      )
    },
    [refresh, sessionId, setError, setPendingPermissionActions],
  )

  const onRewind = useCallback(
    (messageId: string, mode: RewindMode, draft: string) =>
      requestActionJson(
        sessionId,
        'rewind',
        { messageId, mode },
        isRewindResponse,
      ).match(
        (response) => {
          if (!writeComposerDraftForSession(response.sessionId, draft)) {
            setError('Could not keep the selected message as a draft.')
            return false
          }
          setError(null)
          refresh()
          onSelectSession(response.sessionId)
          return true
        },
        (requestError) => {
          setError(requestError.message)
          return false
        },
      ),
    [onSelectSession, refresh, sessionId, setError],
  )

  return { onAction, onRewind }
}

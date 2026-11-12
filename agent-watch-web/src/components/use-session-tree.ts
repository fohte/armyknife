import { useEffect, useState } from 'react'

import { isSessionTreeResponse } from '#components/conversation-pane-response'
import { requestJson } from '#components/conversation-requests'
import type { Session, SessionTreeNode } from '#types'

export type SessionTreeCallbacks = {
  onSessionTreeLoaded?:
    | ((
        tree: readonly SessionTreeNode[],
        selectedSessionId: string,
        refreshKey: number,
        requestSequence: number,
      ) => void)
    | undefined
  onSessionTreeRequestStarted?:
    ((selectedSessionId: string, refreshKey: number) => number) | undefined
  onSessionTreeRequestFailed?:
    | ((
        selectedSessionId: string,
        refreshKey: number,
        requestSequence: number,
        message: string,
      ) => void)
    | undefined
}

type UseSessionTreeOptions = SessionTreeCallbacks & {
  session: Session
  initialSessionTree: readonly SessionTreeNode[] | null
  initialSessionTreeError: string | null
  shouldSkipSessionTreeFetch: boolean
  refreshKey: number
}

export function useSessionTree({
  session,
  initialSessionTree,
  initialSessionTreeError,
  shouldSkipSessionTreeFetch,
  refreshKey,
  onSessionTreeLoaded,
  onSessionTreeRequestStarted,
  onSessionTreeRequestFailed,
}: UseSessionTreeOptions) {
  const [sessionTree, setSessionTree] = useState(initialSessionTree)
  const [sessionTreeError, setSessionTreeError] = useState(
    initialSessionTreeError,
  )

  useEffect(() => {
    if (initialSessionTree !== null) setSessionTree(initialSessionTree)
  }, [initialSessionTree])

  useEffect(() => {
    if (session.hasSessionTree !== true) {
      setSessionTree(null)
      setSessionTreeError(null)
      return
    }
    if (shouldSkipSessionTreeFetch) {
      setSessionTreeError(initialSessionTreeError)
      return
    }
    let current = true
    setSessionTreeError(null)
    const requestSequence =
      onSessionTreeRequestStarted?.(session.id, refreshKey) ?? 0
    void requestJson(
      `/api/sessions/${encodeURIComponent(session.id)}/tree`,
      isSessionTreeResponse,
    ).match(
      (response) => {
        if (!response.sessions.some((node) => node.id === session.id)) {
          const errorMessage =
            'The session tree does not contain the selected session.'
          onSessionTreeRequestFailed?.(
            session.id,
            refreshKey,
            requestSequence,
            errorMessage,
          )
          if (current) setSessionTreeError(errorMessage)
          return
        }
        onSessionTreeLoaded?.(
          response.sessions,
          session.id,
          refreshKey,
          requestSequence,
        )
        if (current) {
          setSessionTree(response.sessions)
          setSessionTreeError(null)
        }
      },
      (treeError) => {
        onSessionTreeRequestFailed?.(
          session.id,
          refreshKey,
          requestSequence,
          treeError.message,
        )
        if (current) setSessionTreeError(treeError.message)
      },
    )
    return () => {
      current = false
    }
  }, [
    initialSessionTreeError,
    onSessionTreeLoaded,
    onSessionTreeRequestFailed,
    onSessionTreeRequestStarted,
    refreshKey,
    session.hasSessionTree,
    session.id,
    shouldSkipSessionTreeFetch,
  ])

  return { sessionTree, sessionTreeError }
}

import { useCallback } from 'react'

import { isRewindResponse } from '#components/conversation-pane-response'
import { writeComposerDraftForSession } from '#components/use-composer-draft'
import type { useSessionOperations } from '#session-operation-context'
import { createSessionOperation } from '#session-operations'
import type { RewindMode } from '#types'

type UseConversationPaneActionsOptions = {
  sessionId: string
  refresh: () => void
  setError: (error: string | null) => void
  onSelectSession: (id: string) => void
  run: ReturnType<typeof useSessionOperations>['run']
}

export function useConversationPaneActions({
  sessionId,
  refresh,
  setError,
  onSelectSession,
  run,
}: UseConversationPaneActionsOptions) {
  const onRewind = useCallback(
    async (messageId: string, mode: RewindMode, draft: string) => {
      const operation = createSessionOperation(sessionId, 'rewind', {
        messageId,
        mode,
      })
      if (operation.isErr()) {
        setError(operation.error.message)
        return false
      }
      const result = await run(operation.value)
      return result.match(
        (value) => {
          if (!isRewindResponse(value)) {
            setError('The server returned an invalid response')
            return false
          }
          const response = value
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
      )
    },
    [onSelectSession, refresh, run, sessionId, setError],
  )

  return { onRewind }
}

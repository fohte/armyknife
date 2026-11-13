import { useCallback } from 'react'

import { isRewindResponse } from '#components/conversation-pane-response'
import { requestActionJson } from '#components/conversation-requests'
import { writeComposerDraftForSession } from '#components/use-composer-draft'
import type { RewindMode } from '#types'

type UseConversationPaneActionsOptions = {
  sessionId: string
  refresh: () => void
  setError: (error: string | null) => void
  onSelectSession: (id: string) => void
}

export function useConversationPaneActions({
  sessionId,
  refresh,
  setError,
  onSelectSession,
}: UseConversationPaneActionsOptions) {
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

  return { onRewind }
}

import { useCallback } from 'react'

import { useOptionalAsyncActionFeedback } from '#async-action-feedback'
import type { AsyncActionOutcome } from '#async-action-model'
import { isRewindResponse } from '#components/conversation-pane-response'
import { writeComposerDraftForSession } from '#components/use-composer-draft'
import type { useSessionOperations } from '#session-operation-context'
import { createSessionOperation } from '#session-operations'
import type { RewindMode } from '#types'

type UseConversationPaneActionsOptions = {
  sessionId: string
  sessionTitle: string
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
  sessionTitle,
}: UseConversationPaneActionsOptions) {
  const actionFeedback = useOptionalAsyncActionFeedback()
  const onRewind = useCallback(
    async (
      messageId: string,
      mode: RewindMode,
      draft: string,
      timelineIndex?: number,
    ) => {
      const operation = createSessionOperation(sessionId, 'rewind', {
        messageId,
        mode,
      })
      if (operation.isErr()) {
        setError(operation.error.message)
        return false
      }
      const runRewind = async (): Promise<AsyncActionOutcome> => {
        const result = await run(operation.value)
        return result.match(
          (value) => {
            if (!isRewindResponse(value)) {
              return {
                status: 'rewindFailed' as const,
                error: 'The server returned an invalid response.',
              }
            }
            const response = value
            if (!writeComposerDraftForSession(response.sessionId, draft)) {
              return {
                status: 'rewindFailed' as const,
                error: 'Could not keep the selected message as a draft.',
              }
            }
            setError(null)
            refresh()
            onSelectSession(response.sessionId)
            return { status: 'rewound' as const }
          },
          (requestError) => ({
            status: 'rewindFailed' as const,
            error: requestError.message,
          }),
        )
      }
      if (actionFeedback !== null) {
        actionFeedback.start({
          kind: 'rewind',
          sessionId,
          sessionTitle,
          ...(timelineIndex === undefined ? {} : { timelineIndex }),
          run: runRewind,
        })
        return true
      }
      const outcome = await runRewind()
      if (outcome.status === 'rewindFailed') setError(outcome.error)
      return outcome.status === 'rewound'
    },
    [
      actionFeedback,
      onSelectSession,
      refresh,
      run,
      sessionId,
      sessionTitle,
      setError,
    ],
  )

  return { onRewind }
}

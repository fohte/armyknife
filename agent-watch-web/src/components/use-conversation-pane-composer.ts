import { useComposerDraft } from '#components/use-composer-draft'
import { useComposerMessage } from '#components/use-composer-message'

type UseConversationPaneComposerOptions = {
  sessionId: string
  messageEnabled: boolean
  externalSending: boolean
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>
}

export function useConversationPaneComposer({
  sessionId,
  messageEnabled,
  externalSending,
  onAction,
}: UseConversationPaneComposerOptions) {
  const draftState = useComposerDraft(sessionId)
  const messageState = useComposerMessage({
    sessionId,
    draft: draftState.draft,
    messageEnabled,
    externalSending,
    clearDraft: draftState.clearDraft,
    restoreDraftIfEmpty: draftState.restoreDraftIfEmpty,
    onAction,
  })

  return { ...draftState, ...messageState }
}

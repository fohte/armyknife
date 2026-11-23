import { useConversationPaneKeyboard } from '#components/use-conversation-pane-keyboard'
import type { NewSessionState } from '#use-new-session'

type UseNewSessionMessageKeyboardOptions = {
  isMobile: boolean
  launchPending: boolean
  onLaunched: (launchId: string) => void
  state: NewSessionState
}

const ignoreInterrupt = () => {}

export function useNewSessionMessageKeyboard({
  isMobile,
  launchPending,
  onLaunched,
  state,
}: UseNewSessionMessageKeyboardOptions) {
  return useConversationPaneKeyboard({
    isMobile,
    canInterrupt: false,
    interruptPending: false,
    interrupt: ignoreInterrupt,
    updateDraft: state.setInitialMessage,
    sendMessage: async () => {
      if (state.selectedDirectory !== null && !launchPending) {
        await state.start(onLaunched, state.engine)
      }
    },
  })
}

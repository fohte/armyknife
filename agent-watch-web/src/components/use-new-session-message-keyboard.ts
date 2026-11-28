import { useConversationPaneKeyboard } from '#components/use-conversation-pane-keyboard'
import type { LaunchSessionRequest } from '#new-session-api'
import type { NewSessionState } from '#use-new-session'

type UseNewSessionMessageKeyboardOptions = {
  isMobile: boolean
  launchPending: boolean
  onStart: (request: LaunchSessionRequest) => void
  state: NewSessionState
}

const ignoreInterrupt = () => {}

export function useNewSessionMessageKeyboard({
  isMobile,
  launchPending,
  onStart,
  state,
}: UseNewSessionMessageKeyboardOptions) {
  return useConversationPaneKeyboard({
    isMobile,
    canInterrupt: false,
    interruptPending: false,
    interrupt: ignoreInterrupt,
    updateDraft: state.setInitialMessage,
    sendMessage: () => {
      if (state.selectedDirectory !== null && !launchPending) {
        state.start(onStart, state.engine)
      }
      return Promise.resolve()
    },
  })
}

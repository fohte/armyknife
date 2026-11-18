import { type KeyboardEvent as ReactKeyboardEvent, useCallback } from 'react'

type UseConversationPaneKeyboardOptions = {
  isMobile: boolean
  canInterrupt: boolean
  interruptPending: boolean
  interrupt: () => void
  updateDraft: (draft: string) => void
  sendMessage: () => Promise<unknown>
}

export function useConversationPaneKeyboard({
  isMobile,
  canInterrupt,
  interruptPending,
  interrupt,
  updateDraft,
  sendMessage,
}: UseConversationPaneKeyboardOptions) {
  return useCallback(
    (event: ReactKeyboardEvent<HTMLTextAreaElement>) => {
      if (
        event.nativeEvent.isComposing ||
        Reflect.get(event.nativeEvent, 'keyCode') === 229
      ) {
        return
      }
      if (event.key === 'Escape' && canInterrupt && !interruptPending) {
        event.preventDefault()
        interrupt()
        return
      }
      if (
        event.key === 'Enter' &&
        !isMobile &&
        event.altKey &&
        !event.shiftKey
      ) {
        event.preventDefault()
        const textarea = event.currentTarget
        textarea.setRangeText(
          '\n',
          textarea.selectionStart,
          textarea.selectionEnd,
          'end',
        )
        updateDraft(textarea.value)
        return
      }
      if (
        event.key === 'Enter' &&
        !isMobile &&
        !event.shiftKey &&
        !event.altKey
      ) {
        event.preventDefault()
        void sendMessage()
      }
    },
    [
      canInterrupt,
      interrupt,
      interruptPending,
      isMobile,
      sendMessage,
      updateDraft,
    ],
  )
}

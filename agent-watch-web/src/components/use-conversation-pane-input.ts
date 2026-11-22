import { useCallback, useRef, useState } from 'react'

import { useConversationPaneKeyboard } from '#components/use-conversation-pane-keyboard'
import { useFileDrop } from '#components/use-file-drop'

type UseConversationPaneInputOptions = {
  isMobile: boolean
  canInterrupt: boolean
  interruptPending: boolean
  onAction: (action: string) => Promise<boolean>
  onClearError: () => void
  onPasteFiles: (files: File[]) => void
  updateDraft: (draft: string) => void
  sendMessage: () => Promise<unknown>
}

export function useConversationPaneInput({
  isMobile,
  canInterrupt,
  interruptPending,
  onAction,
  onClearError,
  onPasteFiles,
  updateDraft,
  sendMessage,
}: UseConversationPaneInputOptions) {
  const interruptInFlight = useRef(false)
  const [isInterrupting, setIsInterrupting] = useState(false)
  const handlePasteFiles = (files: File[]) => {
    onPasteFiles(files)
    onClearError()
  }
  const interrupt = useCallback(() => {
    if (interruptInFlight.current || !canInterrupt) return
    interruptInFlight.current = true
    setIsInterrupting(true)
    const finish = () => {
      interruptInFlight.current = false
      setIsInterrupting(false)
    }
    void onAction('interrupt').then(finish, finish)
  }, [canInterrupt, onAction])
  const { isFileDragActive, onDragEnter, onDragOver, onDragLeave, onDrop } =
    useFileDrop(handlePasteFiles)
  const onComposerKeyDown = useConversationPaneKeyboard({
    isMobile,
    canInterrupt,
    interruptPending,
    interrupt,
    updateDraft,
    sendMessage,
  })

  return {
    handlePasteFiles,
    interrupt,
    isFileDragActive,
    isInterrupting,
    onComposerKeyDown,
    onDragEnter,
    onDragOver,
    onDragLeave,
    onDrop,
  }
}

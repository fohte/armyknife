import type { DragEvent as ReactDragEvent } from 'react'
import { useRef, useState } from 'react'

import { containsFiles, getDroppedFiles } from '#components/file-drop'

export function useConversationFileDrop(onFiles: (files: File[]) => void) {
  const [isFileDragActive, setIsFileDragActive] = useState(false)
  const fileDragDepth = useRef(0)

  const handleDragEnter = (event: ReactDragEvent<HTMLElement>) => {
    if (!containsFiles(event.dataTransfer)) return
    event.preventDefault()
    fileDragDepth.current += 1
    setIsFileDragActive(true)
  }

  const handleDragOver = (event: ReactDragEvent<HTMLElement>) => {
    if (!containsFiles(event.dataTransfer)) return
    event.preventDefault()
    event.dataTransfer.dropEffect = 'copy'
  }

  const handleDragLeave = (event: ReactDragEvent<HTMLElement>) => {
    if (!containsFiles(event.dataTransfer)) return
    fileDragDepth.current = Math.max(0, fileDragDepth.current - 1)
    if (fileDragDepth.current === 0) setIsFileDragActive(false)
  }

  const handleDrop = (event: ReactDragEvent<HTMLElement>) => {
    if (!containsFiles(event.dataTransfer)) return
    event.preventDefault()
    fileDragDepth.current = 0
    setIsFileDragActive(false)
    const files = getDroppedFiles(event.dataTransfer)
    if (files.length > 0) onFiles(files)
  }

  return {
    isFileDragActive,
    handleDragEnter,
    handleDragOver,
    handleDragLeave,
    handleDrop,
  }
}

import type { DragEvent as ReactDragEvent } from 'react'
import { useEffect, useRef, useState } from 'react'

import { containsFiles, getDroppedFiles } from '#components/file-drop'

export function useFileDrop(
  onFiles: (files: File[]) => void,
  enabled = true,
  stopPropagation = false,
) {
  const [isFileDragActive, setIsFileDragActive] = useState(false)
  const fileDragDepth = useRef(0)

  useEffect(() => {
    if (enabled) return
    fileDragDepth.current = 0
    setIsFileDragActive(false)
  }, [enabled])

  const stopEventPropagation = (event: ReactDragEvent<HTMLElement>) => {
    if (stopPropagation) event.stopPropagation()
  }

  const handleDragEnter = (event: ReactDragEvent<HTMLElement>) => {
    if (!enabled || !containsFiles(event.dataTransfer)) return
    event.preventDefault()
    stopEventPropagation(event)
    fileDragDepth.current += 1
    setIsFileDragActive(true)
  }

  const handleDragOver = (event: ReactDragEvent<HTMLElement>) => {
    if (!enabled || !containsFiles(event.dataTransfer)) return
    event.preventDefault()
    stopEventPropagation(event)
    event.dataTransfer.dropEffect = 'copy'
  }

  const handleDragLeave = (event: ReactDragEvent<HTMLElement>) => {
    if (!enabled || !containsFiles(event.dataTransfer)) return
    stopEventPropagation(event)
    fileDragDepth.current = Math.max(0, fileDragDepth.current - 1)
    if (fileDragDepth.current === 0) setIsFileDragActive(false)
  }

  const handleDrop = (event: ReactDragEvent<HTMLElement>) => {
    if (!enabled || !containsFiles(event.dataTransfer)) return
    event.preventDefault()
    stopEventPropagation(event)
    fileDragDepth.current = 0
    setIsFileDragActive(false)
    const files = getDroppedFiles(event.dataTransfer)
    if (files.length > 0) onFiles(files)
  }

  return {
    isFileDragActive: enabled && isFileDragActive,
    onDragEnter: handleDragEnter,
    onDragOver: handleDragOver,
    onDragLeave: handleDragLeave,
    onDrop: handleDrop,
  }
}

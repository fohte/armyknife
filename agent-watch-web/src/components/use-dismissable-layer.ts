import { type RefObject, useEffect } from 'react'

export function useDismissableLayer(
  open: boolean,
  onOpenChange: (open: boolean) => void,
  refs: readonly RefObject<HTMLElement | null>[],
) {
  useEffect(() => {
    if (!open) return

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target
      if (
        target instanceof Node &&
        refs.some((ref) => ref.current?.contains(target) === true)
      ) {
        return
      }
      onOpenChange(false)
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      onOpenChange(false)
    }

    document.addEventListener('pointerdown', handlePointerDown, true)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown, true)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [onOpenChange, open, refs])
}

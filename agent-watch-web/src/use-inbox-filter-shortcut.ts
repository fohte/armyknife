import { useEffect } from 'react'

export function useInboxFilterShortcut(
  dialogOpen: boolean,
  openDialog: () => void,
) {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== '/' || event.defaultPrevented) return
      const target = event.target
      if (
        target instanceof HTMLElement &&
        (target.isContentEditable ||
          target instanceof HTMLInputElement ||
          target instanceof HTMLTextAreaElement ||
          target instanceof HTMLSelectElement)
      ) {
        return
      }
      if (
        !dialogOpen &&
        document.querySelector('[aria-modal="true"]') === null
      ) {
        event.preventDefault()
        openDialog()
      }
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [dialogOpen, openDialog])
}

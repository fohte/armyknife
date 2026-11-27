type QuitLifecycle = {
  beforeQuit: () => void
  rendererPreventedUnload: (discardChanges: boolean) => boolean
  willQuit: () => void
  isQuitting: () => boolean
}

export const createQuitLifecycle = (
  stopRetrying: () => void,
): QuitLifecycle => {
  let isQuitting = false

  return {
    beforeQuit: () => {
      isQuitting = true
    },
    rendererPreventedUnload: (discardChanges) => {
      if (discardChanges) return true

      isQuitting = false
      return false
    },
    willQuit: stopRetrying,
    isQuitting: () => isQuitting,
  }
}

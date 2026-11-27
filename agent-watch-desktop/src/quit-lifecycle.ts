type QuitLifecycle = {
  beforeQuit: () => void
  rendererPreventedUnload: (discardChanges: boolean) => boolean
  willQuit: () => void
  isQuitting: () => boolean
}

type PreventUnloadEvent = {
  preventDefault: () => void
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

export const handleRendererPreventedUnload = (
  lifecycle: QuitLifecycle,
  event: PreventUnloadEvent,
  confirmDiscard: (isQuitting: boolean) => boolean,
) => {
  const discardChanges = confirmDiscard(lifecycle.isQuitting())
  if (lifecycle.rendererPreventedUnload(discardChanges)) {
    event.preventDefault()
  }
}

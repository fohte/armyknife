export type PageVisibilityTarget = {
  visibilityState: DocumentVisibilityState
  addEventListener: (type: 'visibilitychange', listener: EventListener) => void
  removeEventListener: (
    type: 'visibilitychange',
    listener: EventListener,
  ) => void
}

export function isPageVisible(target: PageVisibilityTarget | undefined) {
  return target?.visibilityState !== 'hidden'
}

export function subscribeToPageVisibility(
  target: PageVisibilityTarget | undefined,
  onChange: (visible: boolean) => void,
) {
  if (target === undefined) return () => undefined

  const listener: EventListener = () => {
    onChange(isPageVisible(target))
  }
  target.addEventListener('visibilitychange', listener)
  return () => {
    target.removeEventListener('visibilitychange', listener)
  }
}

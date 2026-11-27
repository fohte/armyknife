export type ConnectionActions = {
  loadWatchPage: () => void
  showWaitingPage: () => void
}

export const applyConnectionProbe = (
  isWaiting: boolean,
  serverResponded: boolean,
  actions: ConnectionActions,
): void => {
  if (serverResponded && isWaiting) actions.loadWatchPage()
  if (!serverResponded && !isWaiting) actions.showWaitingPage()
}

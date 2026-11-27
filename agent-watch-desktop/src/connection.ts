import { isAllowedWatchUrl } from './navigation.js'

export type ConnectionActions = {
  loadWatchPage: () => void
}

export const applyConnectionProbe = (
  isWaiting: boolean,
  serverResponded: boolean,
  actions: ConnectionActions,
): void => {
  if (serverResponded && isWaiting) actions.loadWatchPage()
}

export const resolveRetryUrl = (
  failedUrl: string,
  watchOrigin: string,
  defaultUrl: string,
): string =>
  isAllowedWatchUrl(failedUrl, watchOrigin) ? failedUrl : defaultUrl

import { isAllowedWatchUrl } from './navigation.js'

type ConnectionControllerActions = {
  showWaitingPage: () => void
  replaceWaitingPage: (url: string) => void
}

type ConnectionController = {
  mainFrameDidFail: (url: string) => void
  mainFrameDidLoad: (url: string) => void
  probe: (serverResponded: boolean) => void
}

export const createConnectionController = (
  watchOrigin: string,
  defaultUrl: string,
  actions: ConnectionControllerActions,
): ConnectionController => {
  let state: 'loading' | 'waiting' | 'loaded' = 'loading'
  let retryUrl = defaultUrl

  return {
    mainFrameDidFail: (failedUrl) => {
      if (state === 'waiting') return

      retryUrl = isAllowedWatchUrl(failedUrl, watchOrigin)
        ? failedUrl
        : defaultUrl
      state = 'waiting'
      actions.showWaitingPage()
    },
    mainFrameDidLoad: (loadedUrl) => {
      if (!isAllowedWatchUrl(loadedUrl, watchOrigin)) return

      state = 'loaded'
      retryUrl = defaultUrl
    },
    probe: (serverResponded) => {
      if (state !== 'waiting' || !serverResponded) return

      const url = retryUrl
      state = 'loading'
      actions.replaceWaitingPage(url)
    },
  }
}

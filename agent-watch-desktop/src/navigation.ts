import { Result } from 'neverthrow'

const parseUrl = Result.fromThrowable(
  (rawUrl: string) => new URL(rawUrl),
  () => undefined,
)

export type NavigationAction =
  'allow' | 'open-window' | 'open-external' | 'deny'

export const classifyNavigation = (
  currentUrl: string,
  targetUrl: string,
  watchOrigin: string,
  opensWindow: boolean,
): NavigationAction => {
  const current = parseUrl(currentUrl)
  const target = parseUrl(targetUrl)
  const origin = parseUrl(watchOrigin)

  if (current.isErr() || target.isErr() || origin.isErr()) return 'deny'
  if (opensWindow && current.value.origin !== origin.value.origin) return 'deny'

  if (target.value.origin === origin.value.origin) {
    return opensWindow ? 'open-window' : 'allow'
  }

  return ['http:', 'https:', 'mailto:'].includes(target.value.protocol)
    ? 'open-external'
    : 'deny'
}

export const isAllowedWatchUrl = (
  url: string,
  watchOrigin: string,
): boolean => {
  const parsedUrl = parseUrl(url)
  const parsedOrigin = parseUrl(watchOrigin)

  return (
    parsedUrl.isOk() &&
    parsedOrigin.isOk() &&
    parsedUrl.value.origin === parsedOrigin.value.origin
  )
}

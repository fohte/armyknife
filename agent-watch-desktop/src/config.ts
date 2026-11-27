import { err, ok, Result, type Result as NeverthrowResult } from 'neverthrow'

const DEFAULT_URL = 'http://127.0.0.1:4317'

const parseUrl = Result.fromThrowable(
  (rawUrl: string) => new URL(rawUrl),
  () => 'AGENT_WATCH_URL は有効な URL である必要があります',
)

export type WatchTarget = {
  url: string
  origin: string
}

export const resolveWatchTarget = (
  rawUrl: string | undefined,
): NeverthrowResult<WatchTarget, string> => {
  const url = parseUrl(rawUrl ?? DEFAULT_URL)

  if (url.isErr()) return err(url.error)
  if (
    url.value.protocol !== 'http:' ||
    !['localhost', '127.0.0.1'].includes(url.value.hostname) ||
    url.value.username !== '' ||
    url.value.password !== ''
  ) {
    return err(
      'AGENT_WATCH_URL は localhost または 127.0.0.1 の http URL のみ指定できます',
    )
  }

  return ok({ url: url.value.href, origin: url.value.origin })
}

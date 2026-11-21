import { useCopyToClipboard } from '#components/use-copy-to-clipboard'

export function useCopySessionId(sessionId: string) {
  const { status, copy } = useCopyToClipboard(sessionId)
  return { status, copySessionId: copy }
}

import { useCallback, useEffect, useState } from 'react'

const pendingTimeoutMs = 35_000

export function useSessionResume(
  sessionId: string,
  isEnded: boolean,
  isResuming: boolean,
  onAction: (action: string) => Promise<boolean>,
) {
  const [requestedFor, setRequestedFor] = useState<string | null>(null)
  const isPending = isEnded && (isResuming || requestedFor === sessionId)

  useEffect(() => {
    if (requestedFor !== null && requestedFor !== sessionId) {
      setRequestedFor(null)
      return
    }
    if (!isEnded) {
      setRequestedFor(null)
      return
    }
    if (requestedFor === null) return

    const timeout = window.setTimeout(() => {
      setRequestedFor((current) => (current === sessionId ? null : current))
    }, pendingTimeoutMs)
    return () => {
      window.clearTimeout(timeout)
    }
  }, [isEnded, requestedFor, sessionId])

  const resume = useCallback(() => {
    if (isPending) return
    const requestedSessionId = sessionId
    setRequestedFor(requestedSessionId)
    void onAction('resume').then(
      (success) => {
        if (!success) {
          setRequestedFor((current) =>
            current === requestedSessionId ? null : current,
          )
        }
      },
      () => {
        setRequestedFor((current) =>
          current === requestedSessionId ? null : current,
        )
      },
    )
  }, [isPending, onAction, sessionId])

  return { isPending, resume }
}

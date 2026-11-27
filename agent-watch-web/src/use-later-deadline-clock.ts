import { useEffect, useState } from 'react'

import type { Session } from '#types'

export function useLaterDeadlineClock(sessions: readonly Session[]) {
  const [nowMs, setNowMs] = useState(() => Date.now())
  const nextLaterDeadline = sessions.reduce<number | null>(
    (nextDeadline, session) => {
      if (
        session.category !== 'later' ||
        session.laterUntil === null ||
        session.laterUntil === undefined
      ) {
        return nextDeadline
      }
      const deadline = Date.parse(session.laterUntil)
      if (!Number.isFinite(deadline) || deadline <= nowMs) {
        return nextDeadline
      }
      return nextDeadline === null ? deadline : Math.min(nextDeadline, deadline)
    },
    null,
  )

  useEffect(() => {
    if (nextLaterDeadline === null) return
    const timeout = window.setTimeout(
      () => {
        setNowMs(Date.now())
      },
      Math.max(0, nextLaterDeadline - Date.now()),
    )
    return () => {
      window.clearTimeout(timeout)
    }
  }, [nextLaterDeadline])

  return { nowMs, setNowMs }
}

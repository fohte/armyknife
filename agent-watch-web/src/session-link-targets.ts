import { useRef } from 'react'

import { reuseEqualValue } from '#components/conversation-message-sharing'
import type { Session, SessionLinkTarget } from '#types'

export function useSessionLinkTargets(
  sessions: readonly Session[],
): readonly SessionLinkTarget[] {
  const targetsRef = useRef<readonly SessionLinkTarget[] | null>(null)
  // Link cards only use these fields, so unrelated session updates keep rows stable.
  const nextTargets = sessions.map(
    ({ id, category, isPaused, task, repo, title }) => ({
      id,
      category,
      isPaused,
      task:
        task === null
          ? null
          : { id: task.id, number: task.number, title: task.title },
      repo,
      title,
    }),
  )
  targetsRef.current = reuseEqualValue(targetsRef.current, nextTargets)
  return targetsRef.current
}

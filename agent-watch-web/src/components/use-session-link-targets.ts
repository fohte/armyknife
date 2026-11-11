import { useRef } from 'react'

import { reuseEqualValue } from '#components/conversation-message-sharing'
import type { Session, SessionLinkTarget } from '#types'

export function useSessionLinkTargets(
  sessions: readonly Session[],
): readonly SessionLinkTarget[] {
  // Message rows receive this projection, so omit elapsed time that changes every second.
  const targetsRef = useRef<readonly SessionLinkTarget[] | null>(null)
  const nextTargets = sessions.map(
    ({ id, task, tasks, repo, title, engine, workType, kind, preview }) => ({
      id,
      task:
        task === null
          ? null
          : {
              id: task.id,
              number: task.number,
              title: task.title,
              priority: task.priority,
            },
      linkedTaskCount: tasks?.length ?? (task === null ? 0 : 1),
      repo,
      title,
      engine,
      workType,
      kind,
      preview,
    }),
  )
  targetsRef.current = reuseEqualValue(targetsRef.current, nextTargets)
  return targetsRef.current
}

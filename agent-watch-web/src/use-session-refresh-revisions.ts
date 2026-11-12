import { useCallback, useState } from 'react'

export function useSessionRefreshRevisions() {
  const [state, setState] = useState({
    revision: 0,
    sessionRevisions: new Map<string, number>(),
  })
  const recordSessionChanges = useCallback(
    (changedSessionIds: readonly string[]) => {
      setState((current) => {
        const revision = current.revision + 1
        const sessionRevisions = new Map(current.sessionRevisions)
        for (const sessionId of changedSessionIds) {
          sessionRevisions.set(sessionId, revision)
        }
        return { revision, sessionRevisions }
      })
    },
    [],
  )

  return { sessionRevisions: state.sessionRevisions, recordSessionChanges }
}

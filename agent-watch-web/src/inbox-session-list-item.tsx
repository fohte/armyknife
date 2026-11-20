import { Button } from '@fohte/ui/button'

import { useOptionalAsyncActionFeedback } from '#async-action-feedback'
import { AsyncActionItem } from '#components/async-action-item'
import { SessionRow } from '#components/session-row'
import type { Session } from '#types'

export function InboxSessionListItem({
  session,
  selectedId,
  onSelect,
  isMobile,
}: {
  session: Session
  selectedId: string | null
  onSelect: (id: string) => void
  isMobile: boolean
}) {
  const feedback = useOptionalAsyncActionFeedback()
  const closeAction = feedback?.actions.find(
    (action) => action.kind === 'close' && action.sessionId === session.id,
  )
  return (
    <li className="session-list-item">
      <Button
        type="button"
        variant="plain"
        className={`session-row cursor-pointer category-${session.category}${session.id === selectedId ? ' is-selected' : ''}`}
        data-session-row=""
        aria-current={session.id === selectedId ? 'true' : undefined}
        onClick={() => {
          onSelect(session.id)
        }}
      >
        <SessionRow session={session} density="full" isMobile={isMobile} />
      </Button>
      {closeAction === undefined ? null : (
        <AsyncActionItem
          action={closeAction}
          form="line"
          onRetry={() => feedback?.retry(closeAction.id)}
        />
      )}
    </li>
  )
}

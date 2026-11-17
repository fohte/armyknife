import { Button } from '@fohte/ui/button'

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
    </li>
  )
}

import '#components/session-link.css'

import { Tooltip } from '@base-ui/react/tooltip'
import type { ReactNode } from 'react'

import { getCommandPaletteShortcut } from '#command-palette-shortcut'
import type { SessionLinkTarget } from '#types'

const categoryLabels: Record<string, string> = {
  needs_action: 'Needs action',
  later: 'Later',
  in_progress: 'In progress',
  done: 'Done',
}

function sessionStatus(session: SessionLinkTarget) {
  const category = categoryLabels[session.category] ?? session.category
  return session.isPaused ? `${category} (paused)` : category
}

export type SessionLinkProps = {
  children: ReactNode
  onSelectSession?: ((id: string) => void) | undefined
  open?: boolean
  session: SessionLinkTarget
}

export function SessionLink({
  children,
  onSelectSession,
  open,
  session,
}: SessionLinkProps) {
  const title = session.title ?? children
  const openBesideModifier =
    getCommandPaletteShortcut(navigator.platform).modifier === 'metaKey'
      ? '⌘'
      : 'Ctrl'

  return (
    <Tooltip.Root open={open}>
      <Tooltip.Trigger
        delay={200}
        render={
          <a
            className="session-link-trigger"
            href={`?session=${encodeURIComponent(session.id)}`}
            onClick={(event) => {
              if (
                event.button !== 0 ||
                event.metaKey ||
                event.ctrlKey ||
                event.shiftKey ||
                event.altKey ||
                onSelectSession === undefined
              ) {
                return
              }
              event.preventDefault()
              onSelectSession(session.id)
            }}
          >
            {children}
          </a>
        }
      />
      <Tooltip.Portal>
        <Tooltip.Positioner className="session-link-positioner" sideOffset={8}>
          <Tooltip.Popup className="session-link-card">
            <span className="session-link-card-title">{title}</span>
            <span
              className="session-link-card-state"
              data-category={session.category}
            >
              {sessionStatus(session)}
            </span>
            <span className="session-link-card-task">
              {session.task === null
                ? 'No linked tq task'
                : `tq #${String(session.task.number)} ${session.task.title}`}
            </span>
            <span className="session-link-card-repo">repo {session.repo}</span>
            <span className="session-link-card-hint">
              Click to open · {openBesideModifier}-click to open beside
            </span>
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  )
}

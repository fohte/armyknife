import { Menu } from '@base-ui/react/menu'
import { Check, Clock3, Copy, Ellipsis, Pencil, Star, X } from 'lucide-react'
import { useState } from 'react'

import { useCopySessionId } from '#components/use-copy-session-id'
import { VibeIndicator } from '#components/vibe-indicator'
import type { VibeMode } from '#types'
import { vibeModeShortLabel } from '#vibe-mode'

type MobileConversationActionsMenuProps = {
  sessionId: string
  vibeMode?: VibeMode | undefined
  isDone: boolean
  isStarred: boolean
  open?: boolean
  onLater?: () => void
  onClose?: () => void
  onToggleStarred?: (isStarred: boolean) => void
  onEditTitle: () => void
  onOpenVibe?: () => void
}

export function MobileConversationActionsMenu({
  sessionId,
  vibeMode,
  isDone,
  isStarred,
  open,
  onLater,
  onClose,
  onToggleStarred,
  onEditTitle,
  onOpenVibe,
}: MobileConversationActionsMenuProps) {
  const [isMenuOpen, setIsMenuOpen] = useState(false)
  const { status: copySessionIdStatus, copySessionId } =
    useCopySessionId(sessionId)
  const isOpen = open ?? isMenuOpen

  return (
    <>
      <Menu.Root
        open={isOpen}
        onOpenChange={(nextOpen) => {
          if (open === undefined) setIsMenuOpen(nextOpen)
        }}
      >
        <Menu.Trigger
          className="mobile-conversation-more cursor-pointer"
          type="button"
          aria-label="More actions"
          title="More actions"
        >
          <Ellipsis aria-hidden="true" size={20} />
        </Menu.Trigger>
        <Menu.Portal>
          <Menu.Positioner side="bottom" align="end" sideOffset={10}>
            <Menu.Popup
              className="mobile-conversation-actions-menu"
              aria-label="Session actions"
            >
              {vibeMode === undefined || onOpenVibe === undefined ? null : (
                <Menu.Item
                  className="mobile-conversation-actions-menu-item mobile-conversation-actions-menu-vibe"
                  onClick={onOpenVibe}
                >
                  <VibeIndicator mode={vibeMode} size={16} />
                  <span>Vibe mode</span>
                  <span className="mobile-conversation-actions-menu-state">
                    {vibeModeShortLabel(vibeMode)}
                  </span>
                </Menu.Item>
              )}
              <Menu.Item
                className="mobile-conversation-actions-menu-item"
                onClick={() => onLater?.()}
              >
                <Clock3 aria-hidden="true" size={16} />
                <span>Later</span>
              </Menu.Item>
              {isDone && onClose !== undefined ? (
                <Menu.Item
                  className="mobile-conversation-actions-menu-item"
                  onClick={() => {
                    onClose()
                  }}
                >
                  <X aria-hidden="true" size={16} />
                  <span>Close</span>
                </Menu.Item>
              ) : null}
              <Menu.Item
                className="mobile-conversation-actions-menu-item"
                onClick={() => onToggleStarred?.(!isStarred)}
              >
                <Star
                  aria-hidden="true"
                  className="mobile-conversation-actions-menu-star"
                  fill={isStarred ? 'currentColor' : 'none'}
                  size={16}
                />
                <span>
                  {isStarred ? 'Remove from Starred' : 'Add to Starred'}
                </span>
              </Menu.Item>
              <Menu.Item
                className="mobile-conversation-actions-menu-item"
                onClick={() => {
                  copySessionId()
                }}
              >
                <Copy aria-hidden="true" size={16} />
                <span>
                  {copySessionIdStatus === 'copied'
                    ? 'Session ID copied'
                    : copySessionIdStatus === 'failed'
                      ? 'Copy failed'
                      : 'Copy session ID'}
                </span>
              </Menu.Item>
              <Menu.Item
                className="mobile-conversation-actions-menu-item"
                onClick={onEditTitle}
              >
                <Pencil aria-hidden="true" size={16} />
                <span>Edit title</span>
              </Menu.Item>
              {!isDone && onClose !== undefined ? (
                <Menu.Item
                  className="mobile-conversation-actions-menu-item mobile-conversation-actions-menu-close"
                  onClick={onClose}
                >
                  <X aria-hidden="true" size={16} />
                  <span>Close session…</span>
                </Menu.Item>
              ) : null}
            </Menu.Popup>
          </Menu.Positioner>
        </Menu.Portal>
      </Menu.Root>
      {copySessionIdStatus === 'idle' ? null : (
        <span
          className={`mobile-conversation-copy-status${copySessionIdStatus === 'failed' ? ' is-failed' : ''}`}
          role="status"
        >
          {copySessionIdStatus === 'copied' ? (
            <Check aria-hidden="true" size={15} />
          ) : (
            <X aria-hidden="true" size={15} />
          )}
          {copySessionIdStatus === 'copied'
            ? 'Session ID copied'
            : 'Could not copy session ID'}
        </span>
      )}
    </>
  )
}

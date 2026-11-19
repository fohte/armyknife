import { Menu } from '@base-ui/react/menu'
import { Ellipsis, X } from 'lucide-react'

type DesktopConversationActionsMenuProps = {
  open?: boolean
  onClose: () => void
}

export function DesktopConversationActionsMenu({
  open,
  onClose,
}: DesktopConversationActionsMenuProps) {
  return (
    <Menu.Root open={open}>
      <Menu.Trigger
        aria-label="More session actions"
        className="conversation-more cursor-pointer"
        title="More session actions"
        type="button"
      >
        <Ellipsis aria-hidden="true" size={17} />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Positioner side="bottom" align="end" sideOffset={6}>
          <Menu.Popup className="conversation-actions-menu">
            <Menu.Item
              className="conversation-actions-menu-close"
              onClick={onClose}
            >
              <X aria-hidden="true" size={15} />
              <span>Close session…</span>
            </Menu.Item>
          </Menu.Popup>
        </Menu.Positioner>
      </Menu.Portal>
    </Menu.Root>
  )
}

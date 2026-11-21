import '#components/mobile-navigation-actions.css'

import { Menu, Plus, Search } from 'lucide-react'

import { ActivityButton } from '#components/activity-button'

type MobileNavigationActionsProps = {
  onOpenNavigation: () => void
  onOpenPalette: () => void
  onNewSession?: () => void
}

export function MobileNavigationActions({
  onOpenNavigation,
  onOpenPalette,
  onNewSession = () => undefined,
}: MobileNavigationActionsProps) {
  return (
    <div className="mobile-nav-actions">
      <button
        aria-label="Open navigation"
        className="mobile-nav-action"
        onClick={onOpenNavigation}
        type="button"
      >
        <Menu aria-hidden="true" size={19} />
      </button>
      <button
        aria-label="New session"
        className="mobile-nav-action"
        onClick={() => {
          onNewSession()
        }}
        type="button"
      >
        <Plus aria-hidden="true" size={18} />
      </button>
      <ActivityButton iconOnly />
      <button
        aria-label="Open command palette"
        className="mobile-nav-action"
        onClick={onOpenPalette}
        type="button"
      >
        <Search aria-hidden="true" size={18} />
      </button>
    </div>
  )
}

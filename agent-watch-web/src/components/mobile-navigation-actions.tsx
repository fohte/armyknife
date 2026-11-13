import '#components/mobile-navigation-actions.css'

import { Menu, Search } from 'lucide-react'

type MobileNavigationActionsProps = {
  onOpenNavigation: () => void
  onOpenPalette: () => void
}

export function MobileNavigationActions({
  onOpenNavigation,
  onOpenPalette,
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

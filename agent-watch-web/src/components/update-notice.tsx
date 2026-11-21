import '#components/update-notice.css'

import { RotateCw, X } from 'lucide-react'

type UpdateNoticeProps = {
  version: string
  isMobile: boolean
  onReload: () => void
  onDismiss: () => void
}

export function UpdateNotice({
  version,
  isMobile,
  onReload,
  onDismiss,
}: UpdateNoticeProps) {
  return (
    <div
      aria-label="Application update available"
      className={`update-notice${isMobile ? ' update-notice--mobile' : ''}`}
      role="status"
    >
      <RotateCw aria-hidden="true" className="update-notice-icon" size={14} />
      <span className="update-notice-message">
        New version v{version} is ready.
      </span>
      <button className="update-notice-reload" onClick={onReload} type="button">
        Reload
      </button>
      <button
        aria-label="Dismiss until the next update"
        className="update-notice-dismiss"
        onClick={onDismiss}
        type="button"
      >
        <X aria-hidden="true" size={13} />
      </button>
    </div>
  )
}

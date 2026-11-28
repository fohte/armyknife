import '#components/new-session.css'

import { RotateCcw } from 'lucide-react'

import { formatMessageTime } from '#components/message-time'

type NewSessionRestoredNoticeProps = {
  savedAt: string
  now?: Date
  isMobile: boolean
  onClear: () => void
}

export function NewSessionRestoredNotice({
  savedAt,
  now,
  isMobile,
  onClear,
}: NewSessionRestoredNoticeProps) {
  const time = formatMessageTime(savedAt, now)
  if (time === null) return null

  return (
    <div
      className="new-session-restored-notice"
      data-mobile={isMobile ? 'true' : undefined}
      role="status"
    >
      <RotateCcw aria-hidden="true" size={13} />
      <span>Restored your draft from {time}</span>
      <button onClick={onClear} type="button">
        Clear
      </button>
    </div>
  )
}

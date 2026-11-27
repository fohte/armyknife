import '#components/file-drop-indicator.css'

import { Plus } from 'lucide-react'

export function FileDropIndicator({
  label,
  variant,
  className,
}: {
  label: string
  variant: 'conversation' | 'card'
  className?: string
}) {
  return (
    <div
      className={
        `file-drop-indicator file-drop-indicator--${variant}` +
        (className === undefined ? '' : ` ${className}`)
      }
      role="status"
    >
      {variant === 'conversation' ? (
        <Plus aria-hidden="true" size={22} />
      ) : null}
      <span>{label}</span>
    </div>
  )
}

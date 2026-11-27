import '#components/file-drop-indicator.css'

import { Plus } from 'lucide-react'

export function FileDropIndicator({
  label,
  className,
}: {
  label: string
  className?: string
}) {
  return (
    <div
      className={
        className === undefined
          ? 'file-drop-indicator'
          : `file-drop-indicator ${className}`
      }
      role="status"
    >
      <Plus aria-hidden="true" size={22} />
      <span>{label}</span>
    </div>
  )
}

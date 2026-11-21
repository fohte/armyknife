import '#components/file-drop-indicator.css'

export function FileDropIndicator({ className }: { className?: string }) {
  return (
    <div
      className={
        className === undefined
          ? 'file-drop-indicator'
          : `file-drop-indicator ${className}`
      }
      role="status"
    >
      Drop files to attach
    </div>
  )
}

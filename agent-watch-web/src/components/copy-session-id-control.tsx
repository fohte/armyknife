import { CopySessionIdButton } from '#components/copy-session-id-button'
import { useCopySessionId } from '#components/use-copy-session-id'

type CopySessionIdControlProps = {
  sessionId: string
  compact?: boolean
}

export function CopySessionIdControl({
  sessionId,
  compact = false,
}: CopySessionIdControlProps) {
  const { status, copySessionId } = useCopySessionId(sessionId)

  return (
    <CopySessionIdButton
      status={status}
      compact={compact}
      onCopy={copySessionId}
    />
  )
}

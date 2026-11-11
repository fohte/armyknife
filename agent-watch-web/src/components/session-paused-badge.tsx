import { Chip } from '@fohte/ui/chip'

export function SessionPausedBadge({ isPaused }: { isPaused: boolean }) {
  return isPaused ? <Chip>Paused</Chip> : null
}

import type { Session } from '#types'

export function nextReturnLabel(sessions: readonly Session[]) {
  const timed = sessions
    .map((session) => session.laterUntil)
    .filter((value): value is string => value !== null && value !== undefined)
    .sort((left, right) => Date.parse(left) - Date.parse(right))[0]
  if (timed === undefined) {
    return 'Next activity'
  }
  const deadline = new Date(timed)
  const time = new Intl.DateTimeFormat('en', {
    hour: 'numeric',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(deadline)
  const tomorrow = new Date()
  tomorrow.setDate(tomorrow.getDate() + 1)
  if (deadline.toDateString() === tomorrow.toDateString()) {
    return `Next back tomorrow ${time}`
  }
  if (deadline.getTime() - Date.now() > 86_400_000) {
    const date = new Intl.DateTimeFormat('en', {
      weekday: 'short',
      month: 'numeric',
      day: 'numeric',
    }).format(deadline)
    return `Next back ${date} ${time}`
  }
  return `Next back ${time}`
}

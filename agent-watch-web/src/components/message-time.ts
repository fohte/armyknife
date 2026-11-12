export function formatMessageTime(
  timestamp: string | undefined,
  now: Date = new Date(),
): string | null {
  if (timestamp === undefined) return null

  const date = new Date(timestamp)
  if (Number.isNaN(date.getTime())) return null

  const time = new Intl.DateTimeFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(date)

  if (
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate()
  ) {
    return time
  }

  const month = new Intl.DateTimeFormat('en', { month: 'short' }).format(date)
  const day = String(date.getDate())
  const year = String(date.getFullYear())
  const dateLabel =
    date.getFullYear() === now.getFullYear()
      ? `${month} ${day}`
      : `${month} ${day}, ${year}`

  return `${dateLabel} ${time}`
}

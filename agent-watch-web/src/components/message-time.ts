export function formatMessageTime(
  timestamp: string | undefined,
): string | null {
  if (timestamp === undefined) return null

  const date = new Date(timestamp)
  if (Number.isNaN(date.getTime())) return null

  return new Intl.DateTimeFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(date)
}

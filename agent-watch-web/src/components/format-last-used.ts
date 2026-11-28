export function formatLastUsed(lastUsedAt: string, now: Date): string | null {
  const lastUsedAtMs = Date.parse(lastUsedAt)
  if (Number.isNaN(lastUsedAtMs)) return null

  const elapsedMinutes = Math.max(
    0,
    Math.floor((now.getTime() - lastUsedAtMs) / 60_000),
  )
  if (elapsedMinutes < 1) return 'just now'
  if (elapsedMinutes < 60) return `${String(elapsedMinutes)}m ago`

  const elapsedHours = Math.floor(elapsedMinutes / 60)
  if (elapsedHours < 24) return `${String(elapsedHours)}h ago`
  return `${String(Math.floor(elapsedHours / 24))}d ago`
}

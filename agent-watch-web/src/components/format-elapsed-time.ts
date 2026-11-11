export function formatElapsedTime(totalSeconds: number) {
  if (totalSeconds < 60) return `${String(totalSeconds)}s`
  const minutes = Math.floor(totalSeconds / 60)
  if (minutes < 60) {
    const seconds = totalSeconds % 60
    return seconds === 0
      ? `${String(minutes)}m`
      : `${String(minutes)}m ${String(seconds)}s`
  }
  const hours = Math.floor(minutes / 60)
  return `${String(hours)}h ${String(minutes % 60)}m`
}

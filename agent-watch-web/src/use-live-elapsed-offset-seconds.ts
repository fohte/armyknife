import { usePageClockTime } from '#use-page-clock-time'

export function useLiveElapsedOffsetSeconds(startedAt: number) {
  const nowMs = usePageClockTime(1_000, startedAt)
  return Math.floor(Math.max(0, nowMs - startedAt) / 1_000)
}

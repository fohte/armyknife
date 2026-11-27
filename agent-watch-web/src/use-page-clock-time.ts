import { useSyncExternalStore } from 'react'

import { getPageClockTime, subscribeToPageClock } from '#page-clock'

export function usePageClockTime(precisionMs = 1_000, startedAt = 0) {
  useSyncExternalStore(
    subscribeToPageClock,
    () => Math.floor((getPageClockTime() - startedAt) / precisionMs),
    () => 0,
  )
  return getPageClockTime()
}

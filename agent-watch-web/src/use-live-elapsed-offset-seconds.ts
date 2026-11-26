import { useSyncExternalStore } from 'react'

import { getPageClockTime, subscribeToPageClock } from '#page-clock'

const subscribeWithoutAnchor = () => () => undefined
const getZeroOffsetSeconds = () => 0

export function useLiveElapsedOffsetSeconds(startedAt: number | null) {
  return useSyncExternalStore(
    startedAt === null ? subscribeWithoutAnchor : subscribeToPageClock,
    startedAt === null
      ? getZeroOffsetSeconds
      : () => Math.floor(Math.max(0, getPageClockTime() - startedAt) / 1_000),
    getZeroOffsetSeconds,
  )
}

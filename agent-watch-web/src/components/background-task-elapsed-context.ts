import { createContext, useContext, useSyncExternalStore } from 'react'

import { getPageClockTime, subscribeToPageClock } from '#page-clock'

const BackgroundTaskElapsedStartedAtContext = createContext<number | null>(null)

export const BackgroundTaskElapsedStartedAtProvider =
  BackgroundTaskElapsedStartedAtContext.Provider

export function useBackgroundTaskElapsedStartedAt() {
  return useContext(BackgroundTaskElapsedStartedAtContext)
}

const subscribeWithoutAnchor = () => () => undefined
const getServerOffsetSeconds = () => 0

export function useBackgroundTaskElapsedSeconds(elapsedSeconds: number) {
  const startedAt = useBackgroundTaskElapsedStartedAt()
  const offsetSeconds = useSyncExternalStore(
    startedAt === null ? subscribeWithoutAnchor : subscribeToPageClock,
    startedAt === null
      ? getServerOffsetSeconds
      : () => Math.floor(Math.max(0, getPageClockTime() - startedAt) / 1_000),
    getServerOffsetSeconds,
  )
  return elapsedSeconds + offsetSeconds
}

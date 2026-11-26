import { createContext, useContext } from 'react'

import { useLiveElapsedOffsetSeconds } from '#use-live-elapsed-offset-seconds'

const BackgroundTaskElapsedStartedAtContext = createContext<number | null>(null)

export const BackgroundTaskElapsedStartedAtProvider =
  BackgroundTaskElapsedStartedAtContext.Provider

export function useBackgroundTaskElapsedStartedAt() {
  return useContext(BackgroundTaskElapsedStartedAtContext)
}

export function useBackgroundTaskElapsedSeconds(elapsedSeconds: number) {
  const startedAt = useBackgroundTaskElapsedStartedAt()
  return elapsedSeconds + useLiveElapsedOffsetSeconds(startedAt)
}

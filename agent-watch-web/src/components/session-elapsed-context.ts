import { createContext, useContext } from 'react'

const SessionElapsedStartedAtContext = createContext<number | null>(null)

export const SessionElapsedStartedAtProvider =
  SessionElapsedStartedAtContext.Provider

export function useSessionElapsedStartedAt() {
  return useContext(SessionElapsedStartedAtContext)
}

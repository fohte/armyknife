import { createContext } from 'react'

export const SessionLinkElapsedSecondsContext = createContext<
  ReadonlyMap<string, number>
>(new Map())

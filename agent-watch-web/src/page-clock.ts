import { isPageVisible, type PageVisibilityTarget } from '#session-visibility'

const CLOCK_INTERVAL_MS = 1_000

const listeners = new Set<() => void>()
let nowMs = Date.now()
let intervalId: number | null = null

function notify() {
  nowMs = Date.now()
  for (const listener of listeners) listener()
}

function startClock(target: PageVisibilityTarget) {
  if (!isPageVisible(target) || intervalId !== null) return
  nowMs = Date.now()
  intervalId = window.setInterval(notify, CLOCK_INTERVAL_MS)
}

function stopClock() {
  if (intervalId === null) return
  window.clearInterval(intervalId)
  intervalId = null
}

function handleVisibilityChange() {
  if (typeof document === 'undefined') return
  if (isPageVisible(document)) {
    nowMs = Date.now()
    for (const listener of listeners) listener()
    startClock(document)
  } else {
    stopClock()
  }
}

export function subscribeToPageClock(listener: () => void) {
  if (typeof document === 'undefined') return () => undefined

  if (listeners.size === 0) {
    document.addEventListener('visibilitychange', handleVisibilityChange)
    nowMs = Date.now()
    startClock(document)
  }
  listeners.add(listener)

  return () => {
    listeners.delete(listener)
    if (listeners.size > 0 || typeof document === 'undefined') return
    stopClock()
    document.removeEventListener('visibilitychange', handleVisibilityChange)
  }
}

export function getPageClockTime() {
  return nowMs
}

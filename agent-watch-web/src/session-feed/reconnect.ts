const MAX_RECONNECT_DELAY_MS = 60_000

export function createEventSourceReconnectScheduler(
  initialDelayMs: number,
  reconnect: () => void,
) {
  const delayLimit = Math.min(initialDelayMs, MAX_RECONNECT_DELAY_MS)
  let delayMs = delayLimit
  let timer: ReturnType<typeof setTimeout> | null = null

  const clear = () => {
    if (timer !== null) {
      clearTimeout(timer)
      timer = null
    }
  }

  return {
    clear,
    reset: () => {
      delayMs = delayLimit
      clear()
    },
    schedule: () => {
      if (timer !== null) {
        return
      }

      timer = setTimeout(() => {
        timer = null
        delayMs = Math.min(delayMs * 2, MAX_RECONNECT_DELAY_MS)
        reconnect()
      }, delayMs)
    },
  }
}

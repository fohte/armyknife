import { submitSessionOperation } from '#session-operations'

export function setVibeMode(sessionId: string, enabled: boolean) {
  return submitSessionOperation({ type: 'vibe', sessionId, enabled })
}

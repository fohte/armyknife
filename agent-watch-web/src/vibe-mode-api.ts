import { err, ok } from 'neverthrow'

import { submitSessionOperation } from '#session-operations'
import type { VibeMode } from '#types'
import { isVibeMode } from '#vibe-mode'

export function setVibeMode(sessionId: string, enabled: boolean) {
  return submitSessionOperation({ type: 'vibe', sessionId, enabled }).andThen(
    (response) => {
      if (
        typeof response === 'object' &&
        response !== null &&
        'vibeMode' in response &&
        isVibeMode(response.vibeMode)
      ) {
        return ok<VibeMode, Error>(response.vibeMode)
      }
      return err<VibeMode, Error>(new Error('Invalid vibe mode response'))
    },
  )
}

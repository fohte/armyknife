import { describe, expect, it } from 'vitest'

import {
  appendDesktopUserAgentMarker,
  desktopUserAgentMarker,
} from './user-agent.js'

describe('appendDesktopUserAgentMarker', () => {
  it('preserves the browser identity and identifies Agent Watch Desktop', () => {
    expect(appendDesktopUserAgentMarker('Mozilla/5.0 Electron/44.0')).toBe(
      `Mozilla/5.0 Electron/44.0 ${desktopUserAgentMarker}`,
    )
  })
})

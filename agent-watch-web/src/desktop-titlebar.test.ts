import { describe, expect, it } from 'vitest'

import { applyDesktopTitleBarMarker } from '#desktop-titlebar'

describe('applyDesktopTitleBarMarker', () => {
  it('marks Agent Watch Desktop without changing browser documents', () => {
    const desktopDocument: Pick<HTMLElement, 'dataset'> = { dataset: {} }
    const browserDocument: Pick<HTMLElement, 'dataset'> = { dataset: {} }

    applyDesktopTitleBarMarker(
      'Mozilla/5.0 Electron/44.0 AgentWatchDesktop',
      desktopDocument,
    )
    applyDesktopTitleBarMarker('Mozilla/5.0 Chrome/140.0', browserDocument)

    const snapshot = () => ({
      desktopMarker: desktopDocument.dataset['agentWatchDesktop'] ?? null,
      browserMarker: browserDocument.dataset['agentWatchDesktop'] ?? null,
    })

    expect(snapshot()).toEqual({
      desktopMarker: 'true',
      browserMarker: null,
    })
  })
})

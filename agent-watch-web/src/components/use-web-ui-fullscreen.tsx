import { useEffect, useState } from 'react'

import type { WebUiLinkTarget } from '#components/markdown-session-links'
import { webUiLinkTargetUrl } from '#components/web-ui-link-target'
import { WebUiPane } from '#components/web-ui-pane'

type UseWebUiFullscreenOptions = {
  isMobile: boolean
  statusPanelOpen: boolean
  onSelectTab: (id: string) => void
  onClose: () => void
}

export function useWebUiFullscreen({
  isMobile,
  statusPanelOpen,
  onSelectTab,
  onClose,
}: UseWebUiFullscreenOptions) {
  const [request, setRequest] = useState(0)
  const [webUiRequest, setWebUiRequest] = useState<{
    webUi: WebUiLinkTarget
    url: string
    targetUrl: string | null
  } | null>(null)

  const openWebUi = (webUi: WebUiLinkTarget, url: string) => {
    onSelectTab(
      webUi.name === 'crit'
        ? 'crit'
        : `web-ui-${encodeURIComponent(webUi.url)}`,
    )
    setRequest((current) => current + 1)
    setWebUiRequest({
      webUi,
      url,
      targetUrl: webUiLinkTargetUrl(webUi, url),
    })
  }
  const closeWebUi = () => {
    setWebUiRequest(null)
    onClose()
  }

  useEffect(() => {
    if (!isMobile && !statusPanelOpen) setWebUiRequest(null)
  }, [isMobile, statusPanelOpen])

  const mobileWebUi =
    isMobile && webUiRequest !== null ? (
      <WebUiPane
        name={webUiRequest.webUi.name}
        url={webUiRequest.url}
        exposedUrl={webUiRequest.webUi.exposedUrl ?? null}
        targetUrlOverride={webUiRequest.targetUrl}
        defaultFullscreen
        mobilePresentation
        onClose={closeWebUi}
      />
    ) : null

  return {
    request,
    webUiRequest,
    mobileWebUi,
    openWebUi,
    closeWebUi,
  }
}

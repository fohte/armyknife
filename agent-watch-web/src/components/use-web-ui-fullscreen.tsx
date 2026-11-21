import { useEffect, useState } from 'react'

import type { WebUiLinkTarget } from '#components/markdown-session-links'
import { webUiLinkTargetUrl } from '#components/web-ui-link-target'
import { WebUiPane } from '#components/web-ui-pane'
import { webUiTabId } from '#components/web-ui-tab-id'

type UseWebUiFullscreenOptions = {
  isMobile: boolean
  statusPanelOpen: boolean
  hostname?: string
  onSelectTab: (id: string) => void
  onClose: () => void
}

export function useWebUiFullscreen({
  isMobile,
  statusPanelOpen,
  hostname,
  onSelectTab,
  onClose,
}: UseWebUiFullscreenOptions) {
  const [request, setRequest] = useState(0)
  const [webUiRequest, setWebUiRequest] = useState<{
    webUi: WebUiLinkTarget
    url: string
    targetUrl: string | null
  } | null>(null)

  const openWebUi = (
    webUi: WebUiLinkTarget,
    url: string,
    targetUrl?: string | null,
  ) => {
    onSelectTab(webUiTabId(webUi))
    setRequest((current) => current + 1)
    setWebUiRequest({
      webUi,
      url,
      targetUrl:
        targetUrl === undefined
          ? webUiLinkTargetUrl(webUi, url, hostname)
          : targetUrl,
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
        fullscreenRequest={request}
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

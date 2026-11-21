import type { WebUiLinkTarget } from '#components/markdown-session-links'

export function webUiTabId(webUi: WebUiLinkTarget): string {
  return webUi.name === 'crit'
    ? 'crit'
    : `web-ui-${encodeURIComponent(webUi.url)}`
}

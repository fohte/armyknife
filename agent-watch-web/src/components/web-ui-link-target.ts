import type { WebUiLinkTarget } from '#components/markdown-session-links'
import { webUiTargetUrl } from '#components/web-ui-target-url'

type WebUiLinkMatch = {
  webUi: WebUiLinkTarget
  registeredUrl: URL
}

function parseHttpUrl(value: string): URL | null {
  if (!URL.canParse(value)) return null
  const url = new URL(value)
  return url.protocol === 'http:' || url.protocol === 'https:' ? url : null
}

function hostMatches(left: URL, right: URL): boolean {
  const normalizeHostname = (hostname: string) =>
    hostname === 'localhost' || hostname === '127.0.0.1' ? 'local' : hostname
  return (
    left.protocol === right.protocol &&
    left.port === right.port &&
    normalizeHostname(left.hostname) === normalizeHostname(right.hostname)
  )
}

function pathMatches(pathname: string, basePathname: string): boolean {
  const basePath = basePathname.endsWith('/')
    ? basePathname.slice(0, -1)
    : basePathname
  return (
    pathname === basePath ||
    pathname.startsWith(basePath === '' ? '/' : `${basePath}/`)
  )
}

export function findWebUiLink(
  href: string | undefined,
  webUis: readonly WebUiLinkTarget[],
): WebUiLinkMatch | null {
  if (href === undefined) return null
  const linkUrl = parseHttpUrl(href)
  if (linkUrl === null) return null

  for (const webUi of webUis) {
    for (const candidate of [webUi.url, webUi.exposedUrl]) {
      if (candidate == null || candidate.trim() === '') continue
      const registeredUrl = parseHttpUrl(candidate)
      if (
        registeredUrl !== null &&
        hostMatches(linkUrl, registeredUrl) &&
        pathMatches(linkUrl.pathname, registeredUrl.pathname)
      ) {
        return { webUi, registeredUrl }
      }
    }
  }
  return null
}

export function webUiLinkTargetUrl(
  webUi: WebUiLinkTarget,
  href: string,
  hostname = location.hostname,
): string | null {
  const match = findWebUiLink(href, [webUi])
  const target = webUiTargetUrl(
    { url: webUi.url, exposedUrl: webUi.exposedUrl ?? null },
    hostname,
  )
  const requestedUrl = parseHttpUrl(href)
  const targetUrl = target === null ? null : parseHttpUrl(target)
  if (match === null || requestedUrl === null || targetUrl === null) {
    return null
  }

  const basePath = match.registeredUrl.pathname.endsWith('/')
    ? match.registeredUrl.pathname.slice(0, -1)
    : match.registeredUrl.pathname
  const targetBasePath = targetUrl.pathname.endsWith('/')
    ? targetUrl.pathname.slice(0, -1)
    : targetUrl.pathname
  const suffix = requestedUrl.pathname.slice(basePath.length)
  targetUrl.pathname = `${targetBasePath}${suffix}`
  targetUrl.search = requestedUrl.search
  targetUrl.hash = requestedUrl.hash
  return targetUrl.href
}

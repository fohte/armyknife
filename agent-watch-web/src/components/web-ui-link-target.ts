import type { WebUiLinkTarget } from '#components/markdown-session-links'
import { webUiTargetUrl } from '#components/web-ui-target-url'

type WebUiLinkMatch = {
  webUi: WebUiLinkTarget
  registeredUrl: URL
  isCritPreviewRootAlias: boolean
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
  const basePath = normalizedPathname(basePathname)
  return (
    pathname === basePath ||
    pathname.startsWith(basePath === '' ? '/' : `${basePath}/`)
  )
}

function normalizedPathname(pathname: string): string {
  return pathname.endsWith('/') ? pathname.slice(0, -1) : pathname
}

export function findWebUiLink(
  href: string | undefined,
  webUis: readonly WebUiLinkTarget[],
): WebUiLinkMatch | null {
  if (href === undefined) return null
  const linkUrl = parseHttpUrl(href)
  if (linkUrl === null) return null

  let bestMatch: { match: WebUiLinkMatch; pathLength: number } | null = null
  for (const webUi of webUis) {
    for (const candidate of [webUi.url, webUi.exposedUrl]) {
      if (candidate == null || candidate.trim() === '') continue
      const registeredUrl = parseHttpUrl(candidate)
      const isCritPreviewRootAlias =
        registeredUrl !== null &&
        webUi.name === 'crit' &&
        normalizedPathname(linkUrl.pathname) === '' &&
        normalizedPathname(registeredUrl.pathname) === '/preview'
      if (
        registeredUrl !== null &&
        hostMatches(linkUrl, registeredUrl) &&
        (pathMatches(linkUrl.pathname, registeredUrl.pathname) ||
          isCritPreviewRootAlias)
      ) {
        const pathLength = normalizedPathname(registeredUrl.pathname).length
        if (bestMatch === null || pathLength > bestMatch.pathLength) {
          bestMatch = {
            match: { webUi, registeredUrl, isCritPreviewRootAlias },
            pathLength,
          }
        }
      }
    }
  }
  return bestMatch?.match ?? null
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

  const basePath = normalizedPathname(match.registeredUrl.pathname)
  const targetBasePath = normalizedPathname(targetUrl.pathname)
  const suffix = match.isCritPreviewRootAlias
    ? ''
    : requestedUrl.pathname.slice(basePath.length)
  targetUrl.pathname = `${targetBasePath}${suffix}`
  const requestedKeys = new Set(requestedUrl.searchParams.keys())
  requestedKeys.forEach((key) => {
    targetUrl.searchParams.delete(key)
  })
  requestedUrl.searchParams.forEach((value, key) => {
    targetUrl.searchParams.append(key, value)
  })
  targetUrl.hash = requestedUrl.hash
  return targetUrl.href
}

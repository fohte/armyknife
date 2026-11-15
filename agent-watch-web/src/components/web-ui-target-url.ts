type WebUiTarget = {
  url: string
  exposedUrl?: string | null
}

export function webUiTargetUrl(
  { url, exposedUrl }: WebUiTarget,
  hostname = location.hostname,
): string | null {
  if (hostname === 'localhost' || hostname === '127.0.0.1') {
    return url.replace(
      /^(https?:\/\/)(?:localhost|127\.0\.0\.1)(?=[:/?#]|$)/i,
      (_, protocol: string) => `${protocol}${hostname}`,
    )
  }

  if (
    exposedUrl === undefined ||
    exposedUrl === null ||
    exposedUrl.trim().length === 0
  ) {
    return null
  }

  return exposedUrl
}

export function webUiRelayUrl(
  url: string,
  currentLocation: Pick<Location, 'protocol' | 'hostname'> = window.location,
): string {
  if (!URL.canParse(url)) {
    return url
  }
  const target = new URL(url)

  if (
    target.protocol !== 'http:' ||
    (target.hostname !== 'localhost' && target.hostname !== '127.0.0.1') ||
    target.port === ''
  ) {
    return url
  }

  const relayOrigin = currentLocation.hostname.endsWith('.ts.net')
    ? `https://${currentLocation.hostname}:8443`
    : `${currentLocation.protocol}//${currentLocation.hostname}:4318`
  const relayUrl = new URL(
    `/_web-ui/${target.port}${target.pathname}${target.search}`,
    relayOrigin,
  )
  relayUrl.hash = target.hash
  return relayUrl.toString()
}

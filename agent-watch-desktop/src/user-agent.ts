export const desktopUserAgentMarker = 'AgentWatchDesktop'

export const appendDesktopUserAgentMarker = (userAgent: string) =>
  `${userAgent} ${desktopUserAgentMarker}`

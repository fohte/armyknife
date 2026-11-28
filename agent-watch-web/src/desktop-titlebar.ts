export const applyDesktopTitleBarMarker = (
  userAgent: string,
  documentElement: Pick<HTMLElement, 'dataset'>,
) => {
  if (userAgent.includes('AgentWatchDesktop')) {
    documentElement.dataset['agentWatchDesktop'] = 'true'
  }
}

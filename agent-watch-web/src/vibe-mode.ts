import type { VibeMode } from '#types'

export function isVibeMode(value: unknown): value is VibeMode {
  if (!isRecord(value)) return false
  const mode = value['mode']
  return (
    mode === 'enabled' ||
    mode === 'disabled' ||
    mode === 'repo_disabled' ||
    (mode === 'inherited' && typeof value['ancestorSessionId'] === 'string')
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

export function vibeModeLabel(mode: VibeMode) {
  switch (mode.mode) {
    case 'enabled':
      return 'Vibe on'
    case 'inherited':
      return 'Vibe on · inherited'
    case 'disabled':
      return 'Vibe off'
    case 'repo_disabled':
      return 'Vibe unavailable'
  }
}

export function vibeModeShortLabel(mode: VibeMode) {
  switch (mode.mode) {
    case 'enabled':
      return 'On'
    case 'inherited':
      return 'On · inherited'
    case 'disabled':
      return 'Off'
    case 'repo_disabled':
      return 'Unavailable'
  }
}

export function vibeModeIsOn(mode: VibeMode) {
  return mode.mode === 'enabled' || mode.mode === 'inherited'
}

export function vibeModeReason(mode: VibeMode, repo: string) {
  switch (mode.mode) {
    case 'enabled':
      return `This session can merge pull requests and submit without human review in ${repo}.`
    case 'inherited':
      return `Vibe mode is inherited from ancestor session ${mode.ancestorSessionId}. Turn it on here to keep it enabled if the ancestor turns it off.`
    case 'disabled':
      return `Vibe mode is off for this session. Turning it on allows this session to merge pull requests and submit without human review in ${repo}.`
    case 'repo_disabled':
      return `Vibe mode is disabled by the repository setting for ${repo}. Enable repo.vibe to turn it on.`
  }
}

export function vibeModeChange(mode: VibeMode) {
  const enabled = mode.mode === 'inherited' || mode.mode === 'disabled'
  const message = enabled
    ? 'watch-web からユーザーが vibe mode を on にしました。'
    : 'watch-web からユーザーが vibe mode を off にしました。'
  return {
    enabled,
    message,
    actionLabel:
      mode.mode === 'inherited'
        ? 'Turn on here and send'
        : enabled
          ? 'Turn on and send'
          : 'Turn off and send',
  }
}

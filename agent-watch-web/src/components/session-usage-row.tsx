import '#components/session-usage-row.css'

import {
  formatTokenCount,
  getUsageSeverity,
  type SessionUsage,
} from '#agent-usage'
import { UsagePaceIndicator } from '#components/usage-pace-indicator'

type SessionUsageRowProps = {
  usage: SessionUsage | null
  nowSeconds?: number
  isMobile?: boolean
}

function UsageRate({
  label,
  window,
  nowSeconds,
}: {
  label: string
  window: SessionUsage['rateLimits']['fiveHour']
  nowSeconds: number
}) {
  if (window === null) return null
  const usedPercentage = Math.round(window.usedPercentage)
  const severity = getUsageSeverity(window.usedPercentage)
  return (
    <span className="session-usage-rate">
      {label}{' '}
      <span className={severity === 'normal' ? undefined : `is-${severity}`}>
        {String(usedPercentage)}%
      </span>{' '}
      <UsagePaceIndicator nowSeconds={nowSeconds} window={window} />
    </span>
  )
}

export function SessionUsageRow({
  usage,
  nowSeconds = Math.floor(Date.now() / 1_000),
  isMobile = false,
}: SessionUsageRowProps) {
  if (usage === null) return null
  const context = usage.context
  return (
    <div
      aria-label="Session usage"
      className="session-usage-row"
      data-mobile={isMobile ? 'true' : 'false'}
      role="group"
    >
      <span
        className="session-usage-engine"
        title={usage.engine === 'claude' ? 'Claude Code' : 'Codex'}
      >
        {usage.engine === 'claude' ? 'claude' : 'codex'}
      </span>
      {usage.model !== null ? (
        <span className="session-usage-model">{usage.model}</span>
      ) : null}
      {usage.effort !== null ? (
        <span className="session-usage-effort">({usage.effort})</span>
      ) : null}
      {context !== null ? (
        <>
          <span aria-hidden="true" className="session-usage-separator">
            |
          </span>
          <span className="session-usage-context">
            <span
              aria-label="Context used"
              aria-valuemax={100}
              aria-valuemin={0}
              aria-valuenow={context.usedPercentage}
              className="session-usage-context-meter"
              role="meter"
            >
              <span
                style={{
                  width: `${String(Math.min(100, context.usedPercentage))}%`,
                }}
              />
            </span>
            <span>
              {formatTokenCount(context.usedTokens)} (
              {String(context.usedPercentage)}%)
            </span>
          </span>
        </>
      ) : null}
      {usage.rateLimits.fiveHour !== null ||
      usage.rateLimits.sevenDay !== null ? (
        <span aria-hidden="true" className="session-usage-separator">
          |
        </span>
      ) : null}
      <UsageRate
        label="5h"
        nowSeconds={nowSeconds}
        window={usage.rateLimits.fiveHour}
      />
      <UsageRate
        label="7d"
        nowSeconds={nowSeconds}
        window={usage.rateLimits.sevenDay}
      />
      {usage.version !== null ? (
        <>
          <span aria-hidden="true" className="session-usage-separator">
            |
          </span>
          <span>
            {usage.version.startsWith('v')
              ? usage.version
              : `v${usage.version}`}
          </span>
        </>
      ) : null}
    </div>
  )
}

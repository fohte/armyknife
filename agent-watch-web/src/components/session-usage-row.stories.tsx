import type { Meta, StoryObj } from '@storybook/react-vite'

import type { SessionUsage } from '#agent-usage'
import { SessionUsageRow } from '#components/session-usage-row'

const claudeUsage: SessionUsage = {
  sessionId: 'sample-claude-session',
  engine: 'claude',
  model: 'Sonnet 5.5',
  effort: 'medium',
  context: {
    usedTokens: 84_300,
    windowTokens: 200_000,
    usedPercentage: 42,
  },
  rateLimits: {
    fiveHour: {
      usedPercentage: 41,
      resetsAt: 1_794_841_200,
      windowSeconds: 18_000,
    },
    sevenDay: {
      usedPercentage: 76,
      resetsAt: 1_795_435_200,
      windowSeconds: 604_800,
    },
  },
  version: '2.4.0',
  observedAt: '2026-11-16T12:00:00Z',
}

const codexUsage: SessionUsage = {
  ...claudeUsage,
  sessionId: 'sample-codex-session',
  engine: 'codex',
  model: 'gpt-5-codex',
  effort: 'high',
  context: {
    usedTokens: 51_200,
    windowTokens: 272_000,
    usedPercentage: 19,
  },
  rateLimits: {
    fiveHour: null,
    sevenDay: null,
  },
  version: '0.60.0',
}

const meta = {
  component: SessionUsageRow,
  args: { usage: claudeUsage, nowSeconds: 1_794_832_000 },
  decorators: [
    (Story) => (
      <div style={{ width: '880px', background: '#0a0a0a' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SessionUsageRow>

export default meta
type Story = StoryObj<typeof meta>

export const ClaudeShowsAllUsage: Story = {
  name: 'Claude Code shows model, context, quota, and version',
}

export const CodexShowsContextWithoutSessionQuota: Story = {
  name: 'Codex shows context and version when session quota is absent',
  args: { usage: codexUsage, nowSeconds: 1_794_832_000 },
}

export const QuotaPercentagesShowWarningAndDanger: Story = {
  name: 'Quota percentages show status while window labels stay muted',
  args: {
    usage: {
      ...claudeUsage,
      rateLimits: {
        fiveHour: {
          usedPercentage: 82,
          resetsAt: 1_794_841_200,
          windowSeconds: 18_000,
        },
        sevenDay: {
          usedPercentage: 97,
          resetsAt: 1_795_435_200,
          windowSeconds: 604_800,
        },
      },
    },
    nowSeconds: 1_794_832_000,
  },
}

export const MissingUsageIsHidden: Story = {
  name: 'The row is hidden when the session has no recorded usage',
  args: { usage: null },
}

export const MobileWrapsTheUsageRow: Story = {
  name: 'The row wraps its values on a narrow screen',
  args: { usage: claudeUsage, isMobile: true, nowSeconds: 1_794_832_000 },
  decorators: [
    (Story) => (
      <div style={{ width: '360px', background: '#0a0a0a' }}>
        <Story />
      </div>
    ),
  ],
}

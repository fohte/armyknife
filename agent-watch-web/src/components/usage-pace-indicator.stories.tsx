import type { Meta, StoryObj } from '@storybook/react-vite'

import { UsagePaceIndicator } from '#components/usage-pace-indicator'

const nowSeconds = 1_800_000_000

const meta = {
  component: UsagePaceIndicator,
  args: {
    label: '7d',
    usedPercentage: 30,
    window: {
      usedPercentage: 30,
      resetsAt: nowSeconds + 9_000,
      windowSeconds: 18_000,
    },
    nowSeconds,
  },
  render: (args) => (
    <span>
      <span aria-hidden="true">
        {args.label} {String(Math.round(args.usedPercentage))}%{' '}
      </span>
      <UsagePaceIndicator {...args} />
    </span>
  ),
  decorators: [
    (Story) => (
      <div
        style={{
          background: '#171717',
          fontFamily: 'var(--font-mono)',
          fontSize: '12px',
        }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof UsagePaceIndicator>

export default meta
type Story = StoryObj<typeof meta>

export const OnPace: Story = {
  name: 'The projection stays below 100%',
}

export const AboveQuotaPace: Story = {
  name: 'The projection is from 100% to below 150%',
  args: {
    usedPercentage: 55,
    window: {
      usedPercentage: 55,
      resetsAt: nowSeconds + 9_000,
      windowSeconds: 18_000,
    },
  },
}

export const LikelyToExceedLimit: Story = {
  name: 'The projection reaches 150% or more',
  args: {
    usedPercentage: 80,
    window: {
      usedPercentage: 80,
      resetsAt: nowSeconds + 9_000,
      windowSeconds: 18_000,
    },
  },
}

export const WindowHasElapsedLessThanFivePercent: Story = {
  name: 'No projection appears before 5% of the window has elapsed',
  args: {
    usedPercentage: 3,
    window: {
      usedPercentage: 3,
      resetsAt: nowSeconds + 17_136,
      windowSeconds: 18_000,
    },
  },
}

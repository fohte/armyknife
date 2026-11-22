import type { Decorator, Meta, StoryObj } from '@storybook/react-vite'

import { EndedSessionBanner } from '#components/ended-session-banner'
import { MergeBand, type MergeBandRow } from '#components/merge-band'

const pullRequest = {
  number: 42,
  state: 'Open',
  owner: 'example-org',
  repo: 'example-service',
  title: 'Show delivery status in the session header',
  headSha: 'fixture-head-sha',
  ciState: 'Successful',
  isCurrentBranch: true,
} as const

const failedReason =
  'The base branch changed after confirmation. Refresh the pull request and retry the merge.'

const row = (status: MergeBandRow['status'], error?: string): MergeBandRow => ({
  pullRequest,
  status,
  ...(error === undefined ? {} : { error }),
})

const desktopDecorator: Decorator = (Story) => (
  <div className="conversation-pane" style={{ minHeight: 160, paddingTop: 24 }}>
    <Story />
  </div>
)

const phoneDecorator: Decorator = (Story) => (
  <div
    className="conversation-pane"
    data-mobile="true"
    style={{ width: 390, minHeight: 180, paddingTop: 24 }}
  >
    <Story />
  </div>
)

const meta = {
  component: MergeBand,
  args: {
    rows: [row('ready')],
    isMobile: false,
    onMerge: () => undefined,
    onRetry: () => undefined,
  },
  decorators: [desktopDecorator],
} satisfies Meta<typeof MergeBand>

export default meta
type Story = StoryObj<typeof meta>

export const ReadyToMerge: Story = {
  name: 'A mergeable pull request offers Merge',
}

export const PhoneReadyToMerge: Story = {
  name: 'The phone band offers Merge for a ready pull request',
  args: { isMobile: true },
  decorators: [phoneDecorator],
}

export const MergeIsInProgress: Story = {
  name: 'The band shows that the merge is in progress',
  args: { rows: [row('merging')] },
}

export const PhoneMergeIsInProgress: Story = {
  name: 'The phone band shows that the merge is in progress',
  args: { isMobile: true, rows: [row('merging')] },
  decorators: [phoneDecorator],
}

export const MergeIsQueued: Story = {
  name: 'The band shows that the merge is queued',
  args: { rows: [row('queued')] },
}

export const PhoneMergeIsQueued: Story = {
  name: 'The phone band shows that the merge is queued',
  args: { isMobile: true, rows: [row('queued')] },
  decorators: [phoneDecorator],
}

export const MergeFailedCanBeRetried: Story = {
  name: 'A failed merge shows its reason and a retry action',
  args: { rows: [row('mergeFailed', failedReason)] },
}

export const SeveralPullRequestsCanBeMerged: Story = {
  name: 'The band stacks a row for each mergeable pull request',
  args: {
    rows: [
      row('ready'),
      {
        ...row('ready'),
        pullRequest: {
          ...pullRequest,
          number: 43,
          title: 'Update retry settings',
        },
      },
    ],
  },
}

export const AReadyMergeAppearsAboveAnEndedSession: Story = {
  name: 'The merge band appears above the ended session notice',
  render: (args) => (
    <>
      <MergeBand {...args} />
      <div style={{ marginTop: 12 }}>
        <EndedSessionBanner
          endedAt="2026-01-01T05:32:00.000Z"
          isMobile={false}
          isResuming={false}
          canResume
          onResume={() => undefined}
        />
      </div>
    </>
  ),
}

export const PhoneMergeFailedCanBeRetried: Story = {
  name: 'The phone band truncates a long failure reason',
  args: { isMobile: true, rows: [row('mergeFailed', failedReason)] },
  decorators: [phoneDecorator],
}

export const PhoneCanMergeSeveralPullRequests: Story = {
  name: 'The phone band stacks merge actions for multiple pull requests',
  args: {
    isMobile: true,
    rows: [
      row('ready'),
      {
        ...row('ready'),
        pullRequest: {
          ...pullRequest,
          number: 43,
          title: 'Update retry settings',
        },
      },
    ],
  },
  decorators: [phoneDecorator],
}

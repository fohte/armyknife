import type { Meta, StoryObj } from '@storybook/react-vite'

import { PullRequestListPanel } from '#components/pull-request-list-panel'

const meta = {
  component: PullRequestListPanel,
  args: {
    loading: false,
    error: null,
    pullRequests: [
      {
        number: 2048,
        title: 'Show linked work in the session sidebar',
        url: 'https://github.com/example/project/pull/2048',
        state: 'Open',
        ciState: 'Successful',
        reviewStatus: 'done',
      },
      {
        number: 2046,
        title: 'Update the workspace navigation',
        url: 'https://github.com/example/project/pull/2046',
        state: 'Merged',
        ciState: 'Unknown',
        reviewStatus: 'not_started',
      },
    ],
  },
  decorators: [
    (Story) => (
      <div style={{ width: '360px', height: '520px', display: 'flex' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof PullRequestListPanel>

export default meta
type Story = StoryObj<typeof meta>

export const OpenPullRequestShowsChecksAndReview: Story = {
  name: 'An open pull request shows its CI and review status.',
}

export const NoPullRequestsAreLinked: Story = {
  name: 'The panel explains when no pull requests are linked.',
  args: { pullRequests: [] },
}

export const PullRequestsAreLoading: Story = {
  name: 'The panel shows a loading message before pull requests arrive.',
  args: { pullRequests: [], loading: true },
}

export const PullRequestLoadingFailed: Story = {
  name: 'The panel shows an error when pull requests cannot be loaded.',
  args: { pullRequests: [], error: 'Pull requests could not be loaded.' },
}

export const PartialLookupFailureKeepsLinkedPullRequestsVisible: Story = {
  name: 'The panel keeps linked pull requests visible after a partial failure.',
  args: { error: 'Some linked pull requests could not be loaded.' },
}

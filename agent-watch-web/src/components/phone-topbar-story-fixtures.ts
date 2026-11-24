import type { PullRequest } from '#types'

export const phoneTopbarCurrentPullRequest: PullRequest = {
  number: 2050,
  state: 'Open',
  owner: 'example',
  repo: 'service',
  title: '再送処理を retry() にまとめる',
  url: 'https://github.com/example/service/pull/2050',
  headSha: 'sample-phone-pr-head-sha',
  ciState: 'InProgress',
  isCurrentBranch: true,
}

export const phoneTopbarPullRequests: PullRequest[] = [
  phoneTopbarCurrentPullRequest,
  {
    number: 64,
    state: 'Open',
    owner: 'example',
    repo: 'infra',
    title: '再送キューの上限を設定に出す',
    url: 'https://github.com/example/infra/pull/64',
    headSha: 'sample-phone-pr-head-sha-2',
    ciState: 'Successful',
    isCurrentBranch: false,
  },
  {
    number: 2041,
    state: 'Merged',
    owner: 'example',
    repo: 'service',
    title: '再送のログを整える',
    url: 'https://github.com/example/service/pull/2041',
    headSha: 'sample-phone-pr-head-sha-3',
    ciState: 'Successful',
    isCurrentBranch: false,
  },
]

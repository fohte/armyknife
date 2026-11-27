import type { Meta, StoryObj } from '@storybook/react-vite'

import { CloseChecksList } from '#components/close-checks-list'
import type { CloseWorktreeChecks } from '#session-operation-types'

const exampleChecks: CloseWorktreeChecks = {
  path: '/worktrees/example-project',
  files: ['src/example.ts', 'src/example.test.ts'],
  changes: [
    { status: 'M', path: 'src/example.ts' },
    { status: '??', path: 'src/example.test.ts' },
  ],
  commits: [{ sha: 'abc1234', subject: 'add example implementation' }],
  branch: 'feature/example-work',
  merged: false,
  reason: 'The example pull request is still open.',
  processes: [
    { pid: 12345, name: 'example-server' },
    { pid: 23456, name: 'example-watcher' },
  ],
  processInspectionUnavailable: false,
}

const meta = {
  component: CloseChecksList,
  args: { checks: exampleChecks },
  decorators: [
    (Story) => (
      <div style={{ width: 'min(420px, 100%)' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof CloseChecksList>

export default meta
type Story = StoryObj<typeof meta>

export const UncommittedFilesRemain: Story = {
  name: 'The list marks uncommitted files as remaining.',
  args: {
    checks: {
      ...exampleChecks,
      files: ['src/example.ts', 'src/example.test.ts'],
      merged: true,
      reason: null,
      processes: [],
    },
  },
}

export const BranchIsUnmerged: Story = {
  name: 'The list shows why the branch is not merged.',
  args: {
    checks: {
      ...exampleChecks,
      files: [],
      processes: [],
    },
  },
}

export const ProcessesRemain: Story = {
  name: 'The list names each running process.',
  args: {
    checks: {
      ...exampleChecks,
      files: [],
      merged: true,
      reason: null,
    },
  },
}

export const EveryCheckIsClear: Story = {
  name: 'The list marks clear checks with check icons.',
  args: {
    checks: {
      ...exampleChecks,
      files: [],
      branch: null,
      merged: null,
      reason: null,
      processes: [],
    },
  },
}

export const AllRemainingItemsFitOnAPhone: Story = {
  name: 'The phone layout shows all remaining worktree checks.',
  args: {
    checks: exampleChecks,
  },
  parameters: { viewport: { defaultViewport: 'mobile1' } },
}

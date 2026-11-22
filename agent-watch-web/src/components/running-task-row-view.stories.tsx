import type { Meta, StoryObj } from '@storybook/react-vite'

import { BackgroundTaskRowView } from '#components/running-task-row-view'
import type { BackgroundTask } from '#types'

const meta = {
  component: BackgroundTaskRowView,
  args: {
    task: {
      id: 'fixture-background-task',
      description: 'pnpm test --filter fictional-crate',
      elapsedSeconds: 65,
      kind: 'background_task',
    } satisfies BackgroundTask,
    isOpen: true,
    output: [
      'running 24 tests',
      'test retry::stops_after_limit ... ok',
      'test retry::backs_off ... ok',
    ].join('\n'),
    error: null,
    onToggle: () => undefined,
  },
  decorators: [
    (Story) => (
      <div
        className="session-status-panel"
        data-size="phone"
        style={{ width: '390px', height: '844px', display: 'flex' }}
      >
        <ul
          style={{
            width: '100%',
            margin: 0,
            padding: '16px',
            listStyle: 'none',
          }}
        >
          <Story />
        </ul>
      </div>
    ),
  ],
} satisfies Meta<typeof BackgroundTaskRowView>

export default meta
type Story = StoryObj<typeof meta>

export const BackgroundTaskOutputIsOpen: Story = {
  name: 'The background task shows its loaded output.',
}

export const BackgroundTaskOutputIsClosed: Story = {
  name: 'The background task keeps its output collapsed.',
  args: { isOpen: false, output: null, error: null },
}

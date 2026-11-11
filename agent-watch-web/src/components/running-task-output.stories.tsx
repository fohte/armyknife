import type { Meta, StoryObj } from '@storybook/react-vite'

import { BackgroundTaskOutput } from '#components/running-task-output'

const meta = {
  component: BackgroundTaskOutput,
  args: {
    id: 'fixture-output',
    error: null,
    output:
      'Preparing the synthetic build\nChecking fixture inputs\nBuild complete\n',
  },
  decorators: [
    (Story) => (
      <div style={{ width: '360px', height: '320px' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof BackgroundTaskOutput>

export default meta
type Story = StoryObj<typeof meta>

export const OutputIsRenderedAsPlainText: Story = {
  name: 'The task output keeps its original lines',
}

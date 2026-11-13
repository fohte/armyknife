import type { Meta, StoryObj } from '@storybook/react-vite'

import { WebUiPane } from '#components/web-ui-pane'

const meta = {
  component: WebUiPane,
  args: {
    name: 'crit',
    url: 'http://127.0.0.1:5000/review/sample',
    onClose: () => undefined,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '420px', height: '700px', display: 'flex' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof WebUiPane>

export default meta
type Story = StoryObj<typeof meta>

export const TheCritReviewPaneIsOpen: Story = {
  name: 'The crit review pane is open',
}

export const CritReviewFillsTheScreen: Story = {
  name: 'The crit review fills the screen',
  args: {
    defaultFullscreen: true,
  },
}

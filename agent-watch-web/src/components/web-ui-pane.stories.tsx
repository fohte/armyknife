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

export const CanvasPageFillsTheScreen: Story = {
  name: 'A canvas page fills the screen with its deep link',
  args: {
    name: 'Canvas editor',
    url: 'http://localhost:43123/studio/pages/sample#page-fakehash',
    targetUrlOverride:
      'http://localhost:43123/studio/pages/sample#page-fakehash',
    defaultFullscreen: true,
  },
}

export const MobileCanvasPageHasCloseButton: Story = {
  name: 'A mobile canvas page shows its URL and close button',
  args: {
    name: 'Canvas editor',
    url: 'http://localhost:43123/studio/pages/sample#page-fakehash',
    targetUrlOverride:
      'http://localhost:43123/studio/pages/sample#page-fakehash',
    defaultFullscreen: true,
    mobilePresentation: true,
  },
}

export const MobileWebUiTabShowsCompactHeader: Story = {
  name: 'The phone status web UI tab shows a compact header',
  args: {
    name: 'Preview',
    url: 'https://example.test/dashboard',
    targetUrlOverride: 'https://example.test/dashboard',
    mobileTabPresentation: true,
    previewMessage: 'The web interface is shown here.',
  },
  decorators: [
    (Story) => (
      <div style={{ width: '390px', height: '844px', display: 'flex' }}>
        <Story />
      </div>
    ),
  ],
}

export const RemotePaneOpensTheExposedUrl: Story = {
  name: 'A remote pane opens the exposed URL',
  args: {
    exposedUrl: 'https://review.example.invalid/review/sample',
    hostname: 'watch.example.invalid',
  },
}

export const RemotePaneShowsTheLocalUrlWithoutExposure: Story = {
  name: 'A remote pane shows the local URL when exposure is unavailable',
  args: {
    hostname: 'watch.example.invalid',
  },
}

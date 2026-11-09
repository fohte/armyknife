import '#index.css'

import {
  afterEach,
  beforeEach,
  configureUnhandledApiRequestCheck,
  parameters,
  reportUnhandledApiRequest,
} from '@fohte/storybook-addon/preview'
import { withThemeByClassName } from '@storybook/addon-themes'
import type { Preview } from '@storybook/react-vite'
import { setupWorker } from 'msw/browser'
import { mswLoader } from 'msw-storybook-addon/csf3'

configureUnhandledApiRequestCheck({ pathPrefixes: ['/api/'] })

// @fohte/storybook-addon publishes only a `./preview` subpath export (no
// preset/manager entry), so listing it in main.ts's `addons` never wires its
// beforeEach/afterEach checks — they must be spread into this project's own
// preview annotations to actually run.
const preview: Preview = {
  parameters,
  beforeEach,
  afterEach,
  decorators: [
    withThemeByClassName({
      themes: {
        dark: 'dark',
      },
      defaultTheme: 'dark',
    }),
  ],
  loaders: [
    mswLoader(async () => {
      const worker = setupWorker()
      await worker.start({
        onUnhandledRequest: (request, print) => {
          if (reportUnhandledApiRequest(request.url)) {
            print.error()
          }
        },
      })
      return worker
    }),
  ],
}

export default preview

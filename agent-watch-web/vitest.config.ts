import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { storybookTest } from '@storybook/addon-vitest/vitest-plugin'
import { playwright } from '@vitest/browser-playwright'
import { configDefaults, defineConfig, mergeConfig } from 'vitest/config'

import viteConfig from './vite.config'

const dirname = path.dirname(fileURLToPath(import.meta.url))
const blockExternalRequestsArgs = [
  '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost',
]

function chromiumBrowser() {
  return {
    enabled: true,
    provider: playwright({
      launchOptions: { args: blockExternalRequestsArgs },
    }),
    headless: true,
    instances: [{ browser: 'chromium' as const }],
  }
}

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      projects: [
        {
          extends: true,
          test: {
            name: 'unit',
            exclude: [
              ...configDefaults.exclude,
              'src/App.test.tsx',
              'src/components/agent-watch-nav.test.tsx',
              'src/components/conversation-pane.test.tsx',
              'src/components/message-composer.test.tsx',
            ],
          },
        },
        {
          extends: true,
          test: {
            name: 'app',
            include: [
              'src/App.test.tsx',
              'src/components/agent-watch-nav.test.tsx',
              'src/components/conversation-pane.test.tsx',
              'src/components/message-composer.test.tsx',
            ],
            browser: chromiumBrowser(),
          },
        },
        {
          extends: true,
          plugins: [
            storybookTest({ configDir: path.join(dirname, '.storybook') }),
          ],
          test: {
            name: 'storybook',
            browser: chromiumBrowser(),
            setupFiles: ['./.storybook/vitest.setup.ts'],
          },
        },
      ],
    },
  }),
)

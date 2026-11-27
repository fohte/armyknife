import { defineConfig } from 'tsup'

export default defineConfig({
  entry: ['src/index.ts'],
  format: ['esm'],
  platform: 'node',
  target: 'node22',
  bundle: true,
  external: ['electron'],
  outDir: 'dist',
  clean: true,
})

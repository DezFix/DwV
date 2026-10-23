import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import path from 'path'

export default defineConfig({
  plugins: [svelte()],
  resolve: {
    alias: {
      $lib: path.resolve(__dirname, 'src/lib'),
      $components: path.resolve(__dirname, 'src/components'),
      $stores: path.resolve(__dirname, 'src/stores'),
      $api: path.resolve(__dirname, 'src/api'),
    },
  },
  build: {
    outDir: path.resolve(__dirname, 'src-tauri/frontend'),
    emptyOutDir: true,
  },
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/target/**', '**/src-tauri/gen/**'],
    },
  },
  clearScreen: false,
})

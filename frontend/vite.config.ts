import path from 'path'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(import.meta.dirname, './src'),
    },
  },
  build: {
    rolldownOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('node_modules/three')) return 'three-engine'
          if (id.includes('node_modules/motion') || id.includes('node_modules/@hugeicons')) return 'ui-motion'
          if (id.includes('node_modules/@phosphor-icons')) return 'icons'
        },
      },
    },
  },
  server: {
    proxy: {
      '/api': 'http://127.0.0.1:5055',
    },
  },
})

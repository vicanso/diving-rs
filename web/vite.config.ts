import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react-swc'

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],
  build: {
    chunkSizeWarningLimit: 1024 * 1024,
    rollupOptions: {
      output: {
        manualChunks: {
          common: [
            "axios",
            "pretty-bytes",
          ],
          ui: [
            "react",
            "react-dom",
          ],
          antd: [
            "antd",
          ]
        },
      },
    },
  },
  server: {
    proxy: {
      "/api": {
        target: "http://127.0.0.1:7001",
      },
    },
  }
})

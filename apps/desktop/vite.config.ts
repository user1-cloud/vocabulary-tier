import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';

// Tauri 开发时通过 TAURI_DEV_HOST 暴露给移动端/局域网调试
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig({
  plugins: [svelte(), tailwindcss()],

  // 让 Tauri CLI 能正确识别前端产物目录
  clearScreen: false,

  server: {
    port: 1420,
    // Tauri 需要固定端口，端口被占用时直接失败而不是静默换端口
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: 'ws',
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // src-tauri 由 Rust 侧自己监听，前端不需要扫描
      ignored: ['**/src-tauri/**'],
    },
  },

  // 生产构建：Tauri 使用 WebView2 / WebKit，目标可以放高一些
  build: {
    target: 'esnext',
    minify: process.env.TAURI_ENV_DEBUG ? false : 'esbuild',
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    outDir: 'dist',
    emptyOutDir: true,
  },

  resolve: {
    alias: {
      $lib: fileURLToPath(new URL('./src/lib', import.meta.url)),
    },
  },
});

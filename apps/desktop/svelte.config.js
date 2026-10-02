import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/vite-plugin-svelte').SvelteConfig} */
export default {
  // 使用 Svelte 5 的 runes 语法（本项目不使用 Svelte 4 的 export let / $: / stores）
  preprocess: vitePreprocess(),
  compilerOptions: {
    runes: true,
  },
};

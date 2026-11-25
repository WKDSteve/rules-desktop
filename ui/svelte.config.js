// The Svelte setup of the page: styles and TypeScript in components go through the preprocessor of the Vite plugin.

import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

export default {
  preprocess: vitePreprocess(),
};

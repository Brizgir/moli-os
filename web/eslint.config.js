// The dashboard's lint: ESLint's recommended rules, the Svelte plugin's
// (its accessibility rules among them), browser globals. Formatting is
// Prettier's business, not ESLint's.
import js from '@eslint/js';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';

export default [
  { ignores: ['dist/', 'node_modules/'] },
  js.configs.recommended,
  ...svelte.configs.recommended,
  {
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: 'module',
      globals: { ...globals.browser, ...globals.node },
    },
    rules: {
      // `catch {}` with a comment is how the code says « nothing to do ».
      'no-empty': ['error', { allowEmptyCatch: true }],
      'no-unused-vars': ['error', { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrors: 'none' }],
      // A `{' · '}` keeps a space Svelte would otherwise drop between blocks.
      'svelte/no-useless-mustaches': 'off',
      // Plain Map and Set outside reactive state are fine; the rule cannot
      // tell them apart from the ones that should be SvelteMap.
      'svelte/prefer-svelte-reactivity': 'off',
    },
  },
  {
    files: ['**/*.test.js'],
    languageOptions: { globals: { ...globals.node } },
  },
];

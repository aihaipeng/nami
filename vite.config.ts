import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: { target: 'safari16' },
  server: { watch: { ignored: ['**/src-tauri/**'] } },
  test: {
    projects: [
      { test: { name: 'ui', include: ['tests/frontend/history-ui.test.tsx', 'tests/frontend/unified-records-ui.test.tsx', 'tests/frontend/clipboard-settings-ui.test.tsx', 'tests/frontend/clipboard-ui.test.tsx', 'tests/frontend/translation-ui.test.tsx', 'src/shared/presentation.test.tsx'] } },
      { test: { name: 'functional', include: ['tests/frontend/translation-service-ui.test.tsx', 'src/translation/history.test.ts', 'src/shared/source.test.ts'] } },
      { test: { name: 'dfx-reliability', include: ['tests/frontend/native-bridge-ui.test.tsx', 'tests/frontend/history-pagination-ui.test.tsx', 'tests/frontend/translation-reliability.test.tsx'] } },
      { test: { name: 'dfx-security', include: ['tests/frontend/translation-security.test.tsx', 'src/shared/input.test.ts'] } },
      { test: { name: 'dfx-compatibility', include: ['tests/frontend/platform-history-ui.test.tsx', 'src/shared/locale.test.ts'] } },
    ],
  },
});

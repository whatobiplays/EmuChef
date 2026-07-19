import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    include: ["tests/**/*.dom.test.tsx"],
    restoreMocks: true,
    setupFiles: ["tests/vitest.setup.ts"],
  },
});

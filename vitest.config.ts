import * as path from "node:path";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: [
      {
        find: /^@t3tools\/contracts$/,
        replacement: path.resolve(import.meta.dirname, "./packages/contracts/src/index.ts"),
      },
      {
        find: /^@t3tools\/shared\/(.*)$/,
        replacement: path.resolve(import.meta.dirname, "./packages/shared/src/$1.ts"),
      },
    ],
  },
});

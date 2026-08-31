import { chmodSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

let packageDir;
try {
  packageDir = dirname(require.resolve("node-pty/package.json"));
} catch {
  process.exit(0);
}

const helperPaths = [
  join(packageDir, "build", "Release", "spawn-helper"),
  join(packageDir, "prebuilds", `${process.platform}-${process.arch}`, "spawn-helper"),
];

for (const helperPath of helperPaths) {
  if (!existsSync(helperPath)) {
    continue;
  }
  chmodSync(helperPath, 0o755);
}

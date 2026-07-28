import { cp, mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(new URL("..", import.meta.url).pathname);
const output = resolve(root, "dist/extensions");
await mkdir(output, { recursive: true });

for (const browser of ["chromium", "firefox"]) {
  const packageRoot = resolve(root, `packages/extension-${browser}`);
  const target = resolve(output, browser);
  await mkdir(target, { recursive: true });
  await cp(resolve(packageRoot, "public"), target, { recursive: true });
  await cp(resolve(packageRoot, "dist"), resolve(target, "dist"), { recursive: true });
  await cp(resolve(root, "packages/extension-content/dist"), resolve(target, "content"), { recursive: true });
  await writeFile(resolve(target, "BUILD.txt"), `FCast Web Sender ${browser} extension\n`);
}

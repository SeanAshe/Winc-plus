// Release assets are pinned by version/asset ID and SHA-256, including the AppImage runtime.
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const tools = [
  ["linuxdeploy.AppImage", "https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage", "c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d"],
  ["appimagetool.AppImage", "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage", "ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"],
  ["runtime-x86_64", "https://api.github.com/repos/AppImage/type2-runtime/releases/assets/657374641", "617aa77314d57c8b8e4fae9c3f0b35af7b30d5529cb69e98f64f92510ed98a46"],
];
// Token for GitHub API downloads.
const token = process.env.GITHUB_TOKEN;
const directory = resolve("target/release-tools");
mkdirSync(directory, { recursive: true });
for (const [name, url, digest] of tools) {
  const file = resolve(directory, name);
  const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
  if (!existsSync(file) || hash(readFileSync(file)) !== digest) {
    const headers = { Accept: "application/octet-stream" };
    if (token && new URL(url).hostname === "api.github.com") headers.Authorization = `Bearer ${token}`;
    const response = await fetch(url, { headers, signal: AbortSignal.timeout(120_000) });
    if (!response.ok) {
      const detail = (await response.text().catch(() => "")).slice(0, 300);
      throw new Error(`${name}: download failed (${response.status}) ${detail}`);
    }
    const bytes = Buffer.from(await response.arrayBuffer());
    if (hash(bytes) !== digest) throw new Error(`${name}: SHA-256 mismatch`);
    writeFileSync(`${file}.part`, bytes);
    renameSync(`${file}.part`, file);
  }
  chmodSync(file, 0o755);
}

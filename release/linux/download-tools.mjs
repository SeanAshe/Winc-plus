// Release assets are pinned by version/asset ID and SHA-256, including the AppImage runtime.
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

// The tools run on the machine that packages, and the runtime is the one of the packaged
// architecture: both are those of this machine, since each architecture is packaged natively.
const pinned = {
  x64: [
    ["linuxdeploy.AppImage", "https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage", "c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d"],
    ["appimagetool.AppImage", "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage", "ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"],
    ["runtime", "https://api.github.com/repos/AppImage/type2-runtime/releases/assets/657374641", "617aa77314d57c8b8e4fae9c3f0b35af7b30d5529cb69e98f64f92510ed98a46"],
  ],
  arm64: [
    ["linuxdeploy.AppImage", "https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-aarch64.AppImage", "620095110d693282b8ebeb244a95b5e911cf8f65f76c88b4b47d16ae6346fcff"],
    ["appimagetool.AppImage", "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-aarch64.AppImage", "f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158"],
    ["runtime", "https://api.github.com/repos/AppImage/type2-runtime/releases/assets/657373827", "9a72a744d221576c9347ec074243acc5c3c7568896b12714e0e82e9f0bf47900"],
  ],
};
const tools = pinned[process.arch];
if (!tools) throw new Error(`no packaging tools for ${process.arch}`);
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

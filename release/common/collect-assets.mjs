// Stage verified native bundles under their public release file names.
import { copyFileSync, existsSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { msiVersion, version as currentVersion } from "./config.mjs";

// Each platform's files: [bundle directory, file suffix, role]. "update" payloads are listed in the
// platform's signed appcast; "feed" is the appcast itself, under its fixed name.
const specifications = {
  "windows-x64": [
    ["nsis", "-setup.exe", "update"], ["msi", ".msi", "update"], ["portable", "-portable.zip", "download"],
    ["updates", "appcast-windows.xml", "feed"],
  ],
};
export const releasePlatforms = Object.keys(specifications);

export function releaseAssets(platform, version) {
  msiVersion(version);
  if (!specifications[platform]) throw new Error(`unsupported release platform: ${platform}`);
  const assets = specifications[platform].map(([directory, suffix, role]) => ({
    directory, extension: suffix, update: role === "update", feed: role === "feed",
    name: role === "feed" ? suffix : `Winc-${version}-${platform}${suffix}`,
  }));
  if (platform === "linux-x64" && /^\d+\.\d+\.\d+$/.test(version)) {
    assets.push({
      directory: "arch", extension: ".pkg.tar.zst", update: false, feed: false,
      name: `winc-${version}-1-x86_64.pkg.tar.zst`,
    });
  }
  return assets;
}

export function requireFile(file) {
  if (!existsSync(file) || !statSync(file).isFile() || !statSync(file).size) throw new Error(`missing or empty release asset: ${file}`);
}

function collectReleaseAssets({ platform, version, bundle, out }) {
  const plan = [];
  for (const spec of releaseAssets(platform, version)) {
    const directory = join(bundle, spec.directory);
    const matches = existsSync(directory) ? readdirSync(directory).filter((name) => spec.feed ? name === spec.name : name.endsWith(spec.extension)) : [];
    if (matches.length !== 1 || matches[0] !== spec.name) throw new Error(`expected one ${spec.name} in ${directory}`);
    plan.push({ from: join(directory, spec.name), to: join(out, spec.name) });
  }
  for (const { from, to } of plan) {
    requireFile(from);
    if (existsSync(to)) throw new Error(`release asset already staged: ${to}`);
  }
  mkdirSync(out, { recursive: true });
  for (const { from, to } of plan) copyFileSync(from, to);
  return plan.map(({ to }) => to);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const args = process.argv.slice(2);
    const value = (flag) => args.includes(flag) ? args[args.indexOf(flag) + 1] : undefined;
    for (const flag of ["--platform", "--bundle", "--out"]) if (!value(flag)) throw new Error(`missing ${flag}`);
    console.log(collectReleaseAssets({ platform: value("--platform"), version: currentVersion(), bundle: value("--bundle"), out: value("--out") }).join("\n"));
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}

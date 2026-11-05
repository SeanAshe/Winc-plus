// Release build inputs: update channel and public key, signing thumbprint, and MSI version.
import { appendFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { publicKey, signingKey } from "./update-signing.mjs";
import { workspaceVersion } from "./version.mjs";

export const version = () => workspaceVersion();

export function msiVersion(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(version);
  if (!match) throw new Error(`not a semantic version: ${version}`);
  const [major, minor, patch] = match.slice(1, 4).map(Number);
  if (major > 255 || minor > 255 || patch > 65535) throw new Error(`${version} exceeds the MSI limits (255.255.65535)`);
  const core = `${major}.${minor}.${patch}`;
  if (!match[4]) return core;
  const numbers = match[4].split(".").filter((part) => /^\d+$/.test(part));
  const revision = numbers.length ? Number(numbers.at(-1)) : 0;
  if (revision > 65535) throw new Error(`${version}: prerelease number exceeds 65535`);
  return `${core}.${revision}`;
}

// In-app updates read the appcast and packages from this repository's GitHub Releases.
// `releases/latest` is the newest non-prerelease, so stable and beta builds check the same feed.
export const channelOf = (version) => version.includes("-") ? "beta" : "stable";
export const channelBase = (_version) => "https://github.com/SeanAshe/Winc-plus/releases/latest/download";

function releaseConfig({ version, thumbprint, key }) {
  if (thumbprint && !/^[0-9A-Fa-f]{40}$/.test(thumbprint)) throw new Error("--thumbprint must be a 40-digit SHA-1 certificate thumbprint");
  return {
    version, channel: channelOf(version), productName: "winc", identifier: "com.deeix.winc",
    msiVersion: msiVersion(version), certificateThumbprint: thumbprint?.toUpperCase() ?? "",
    updateChannels: `${channelOf(version)}=${channelBase(version)}`, updatePublicKey: publicKey(key),
  };
}

function main() {
  const args = process.argv.slice(2);
  const value = (flag) => args.includes(flag) ? args[args.indexOf(flag) + 1] : undefined;
  const config = releaseConfig({ version: version(), thumbprint: value("--thumbprint"), key: signingKey() });
  if (value("--output")) writeFileSync(value("--output"), `${JSON.stringify(config)}\n`);
  else process.stdout.write(`${JSON.stringify(config)}\n`);
  if (process.env.GITHUB_ENV) {
    appendFileSync(process.env.GITHUB_ENV,
      `WINC_UPDATE_CHANNELS=${config.updateChannels}\nWINC_UPDATE_PUBLIC_KEY=${config.updatePublicKey}\n`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}

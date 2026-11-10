// The upstream version is the single line of the VERSION file. This fork's version is the integer
// in release/fork-version: commits ahead of DEEIX-AI/Winc dev, written by release/bump-fork-version.sh.
// The release tag is `<upstream>_<fork>`. Run directly to print the upstream version, or pass --tag
// to print the release tag.
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

export function workspaceVersion(directory = root) {
  const version = readFileSync(join(directory, "VERSION"), "utf8").trim();
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error(`VERSION is not a semantic version: ${version}`);
  }
  return version;
}

// Commits ahead of DEEIX-AI/Winc dev. The commit hook writes this file.
export function forkVersion(directory = root) {
  const file = join(directory, "release/fork-version");
  const count = readFileSync(file, "utf8").trim();
  if (!/^[1-9]\d*$/.test(count)) throw new Error(`release/fork-version must be a positive integer: ${count}`);
  return count;
}

export function releaseTag(directory = root) {
  return `${workspaceVersion(directory)}_${forkVersion(directory)}`;
}

export function isReleaseTag(tag, version) {
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`^${escaped}_[1-9]\\d*$`).test(tag);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    console.log(process.argv.includes("--tag") ? releaseTag() : workspaceVersion());
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

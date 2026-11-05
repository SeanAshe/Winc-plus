// Assemble all four verified targets, write SHA256SUMS, then upload once into one draft release.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFileSync, createReadStream, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

import { releaseAssets, releasePlatforms, requireFile } from "./collect-assets.mjs";
import { workspaceVersion } from "./version.mjs";

// SHA256SUMS uses the GNU coreutils format (`sha256sum -c SHA256SUMS`; on Windows compare with
// `Get-FileHash <file> -Algorithm SHA256`).
const CHECKSUMS_FILE = "SHA256SUMS";

function hashFile(path) {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    createReadStream(path)
      .on("error", reject)
      .on("data", (chunk) => hash.update(chunk))
      .on("end", () => resolve(hash.digest("hex")));
  });
}

/** `<hex>  <name>` lines sorted by name; the checksums file never lists itself. */
function formatChecksums(entries) {
  const lines = entries
    .filter(({ name }) => name !== CHECKSUMS_FILE)
    .map(({ name, sha256 }) => {
      if (!/^[0-9a-f]{64}$/.test(sha256)) {
        throw new Error(`${name}: not a SHA-256 digest`);
      }
      if (/[\r\n\\]/.test(name)) {
        throw new Error(`${name}: unsupported character in asset name`);
      }
      return { name, line: `${sha256}  ${name}` };
    })
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  return lines.map(({ line }) => `${line}\n`).join("");
}

// Every platform's files must be present, and nothing else.
function releaseFiles({ version, tag, directory }) {
  if (tag !== `v${version}`) throw new Error(`Tag ${tag} does not match the Cargo.toml version ${version}`);
  const files = releasePlatforms.flatMap((platform) => releaseAssets(platform, version).map((spec) => spec.name));
  for (const name of files) requireFile(join(directory, name));
  const allowed = new Set([...files, CHECKSUMS_FILE]);
  for (const name of readdirSync(directory)) {
    if (!allowed.has(name)) throw new Error(`unexpected release asset: ${name}`);
  }
  return files.sort();
}

async function assembleRelease(options) {
  const files = releaseFiles(options);
  const entries = await Promise.all(files.map(async (name) => ({ name, sha256: await hashFile(join(options.directory, name)) })));
  writeFileSync(join(options.directory, CHECKSUMS_FILE), formatChecksums(entries));
  return [...files, CHECKSUMS_FILE];
}

function existingDraft(releases, tag) {
  const matches = releases.filter((release) => release.tag_name === tag);
  if (matches.length > 1) throw new Error(`${tag} has multiple releases; remove the duplicate drafts before retrying`);
  if (matches[0] && !matches[0].draft) throw new Error(`${tag} is already published; refusing to replace its assets`);
  return matches[0];
}

function releaseNotes(version) {
  const stem = `Winc-${version}`;
  return `### ⏬ Download

| Platform | Download |
| --- | --- |
| macOS · Apple silicon | \`${stem}-macos-arm64.dmg\` |
| macOS · Intel | \`${stem}-macos-x64.dmg\` |
| Linux · x86_64 | \`${stem}-linux-x64.AppImage\` or \`.deb\` |
| Windows · x86_64 | \`${stem}-windows-x64-setup.exe\` |
| Windows · x86_64 · MSI | \`${stem}-windows-x64.msi\` |
| Windows · x86_64 · Portable | \`${stem}-windows-x64-portable.zip\` |

Checksums are in \`SHA256SUMS\`. The signed \`appcast-*.xml\` files are the in-app update feeds served from this repository's GitHub Releases.
`;
}

async function main() {
  const args = process.argv.slice(2);
  const value = (flag) => args[args.indexOf(flag) + 1];
  for (const flag of ["--tag", "--sha", "--dir"]) {
    if (!args.includes(flag) || !value(flag)) throw new Error(`missing ${flag}`);
  }
  const tag = value("--tag");
  const sha = value("--sha");
  if (!/^[0-9a-f]{40}$/.test(sha)) throw new Error("--sha must be an immutable Git commit");
  const repository = process.env.GH_REPO;
  const directory = value("--dir");
  const version = workspaceVersion(process.cwd());
  const files = await assembleRelease({ version, tag, directory });
  // A dry run exercises manifest/checksum generation without a GitHub write.
  if (args.includes("--dry-run")) {
    console.log(files.join("\n"));
    return;
  }
  const gh = (args, input) => execFileSync("gh", args, { encoding: "utf8", input, maxBuffer: 16 * 1024 * 1024 });
  // Resolve the actual tag, including annotated tags, never a same-named branch.
  const commit = JSON.parse(gh(["api", `repos/${repository}/commits/${encodeURIComponent(`refs/tags/${tag}`)}`]));
  if (commit.sha !== sha) throw new Error(`${tag} moved since the build started (${sha} -> ${commit.sha})`);
  const pages = JSON.parse(gh(["api", `repos/${repository}/releases?per_page=100`, "--paginate", "--slurp"]));
  let release = existingDraft(pages.flat(), tag);
  const unexpected = (release?.assets ?? []).filter(({ name }) => !files.includes(name));
  if (unexpected.length) {
    throw new Error(`draft contains assets outside this build; remove or reconcile them before retrying: ${unexpected.map(({ name }) => name).join(", ")}`);
  }
  if (!release) {
    // The tag already exists and was verified above. An explicit target_commitish
    // can require workflow-write permission, which GITHUB_TOKEN cannot grant.
    release = JSON.parse(gh(["api", "--method", "POST", `repos/${repository}/releases`, "--input", "-"], JSON.stringify({
      tag_name: tag, name: tag, draft: true,
      prerelease: version.includes("-"), body: releaseNotes(version),
    })));
  }
  gh(["release", "upload", tag, ...files.map((name) => join(directory, name)), "--repo", repository, "--clobber"]);
  console.log(`Draft release ready: ${release.html_url}`);
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `Draft release: ${release.html_url}\n\nBuilt from ${sha}; uploaded ${files.length} files.\n`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}

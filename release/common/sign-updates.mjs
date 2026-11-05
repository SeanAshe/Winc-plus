// Sign native payloads and Sparkle feeds before a platform's artifact is uploaded.
import { mkdirSync, readFileSync, writeFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { releaseAssets, requireFile } from "./collect-assets.mjs";
import { channelBase, version as currentVersion } from "./config.mjs";
import { signingKey, signBytes, signFeed, verifyFeed, verificationKey, publicKey } from "./update-signing.mjs";

const escape = (value) => String(value).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");

// Download URLs point at this repository's GitHub Releases (latest/download).
function signUpdates({ platform, version, bundle, key }) {
  const specs = releaseAssets(platform, version);
  const packages = specs.filter((spec) => spec.update);
  for (const spec of packages) requireFile(join(bundle, spec.directory, spec.name));
  const items = [];
  for (const spec of packages) {
    const file = join(bundle, spec.directory, spec.name);
    const signature = signBytes(readFileSync(file), key);
    const os = platform.startsWith("macos") ? "macos" : platform.split("-")[0];
    const arch = platform === "macos-arm64" ? "arm64" : "x86_64";
    const installer = os === "windows" ? ` installer="${spec.directory}"` : "";
    const url = `${channelBase(version)}/${encodeURIComponent(spec.name)}`;
    items.push(`    <item>\n      <title>${escape(version)}</title>\n      <sparkle:version>${escape(version)}</sparkle:version>\n      <sparkle:shortVersionString>${escape(version)}</sparkle:shortVersionString>\n${os === "macos" ? "      <sparkle:minimumSystemVersion>11.0</sparkle:minimumSystemVersion>\n" : ""}      <enclosure url="${escape(url)}" length="${statSync(file).size}" type="application/octet-stream" sparkle:os="${os}" arch="${arch}"${installer} sparkle:edSignature="${signature}"/>\n    </item>\n`);
  }
  const xml = `<?xml version="1.0" encoding="utf-8"?>\n<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">\n  <channel>\n    <title>winc</title>\n${items.join("")}  </channel>\n</rss>\n`;
  const feed = signFeed(xml, key);
  if (!verifyFeed(feed, verificationKey(publicKey(key)))) throw new Error("Invalid appcast signature");
  const spec = specs.find((spec) => spec.feed);
  mkdirSync(join(bundle, spec.directory), { recursive: true });
  writeFileSync(join(bundle, spec.directory, spec.name), feed);
  return feed;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const args = process.argv.slice(2);
    const value = (flag) => args.includes(flag) ? args[args.indexOf(flag) + 1] : undefined;
    if (!value("--platform") || !value("--bundle")) throw new Error("--platform and --bundle are required");
    signUpdates({ platform: value("--platform"), version: currentVersion(), bundle: value("--bundle"), key: signingKey() });
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}

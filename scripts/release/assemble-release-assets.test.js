#!/usr/bin/env node

const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  allReleaseAssetNames,
  BUNDLE_ASSET_NAMES,
  BUNDLE_CHECKSUM_MANIFEST,
  CHECKSUM_MANIFEST,
  checksummedReleaseAssetNames,
} = require("../../npm/codewhale/scripts/artifacts");
const {
  assemble,
  parseChecksumManifest,
  verifyAssetDirectory,
  windowsLauncherContents,
} = require("./assemble-release-assets");

const repoRoot = path.resolve(__dirname, "..", "..");

function sha256(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function makeIntermediateArtifacts(root) {
  const generated = new Set(["codewhale.bat", CHECKSUM_MANIFEST]);
  const copied = allReleaseAssetNames().filter((name) => !generated.has(name));
  for (const name of copied) {
    if (name === BUNDLE_CHECKSUM_MANIFEST) {
      continue;
    }
    const artifactDirectory = BUNDLE_ASSET_NAMES.includes(name)
      ? path.join(root, "codewhale-bundles")
      : path.join(root, name);
    fs.mkdirSync(artifactDirectory, { recursive: true });
    fs.writeFileSync(path.join(artifactDirectory, name), `fixture:${name}\n`);
  }

  const bundleManifestDirectory = path.join(root, "codewhale-bundles");
  fs.mkdirSync(bundleManifestDirectory, { recursive: true });
  const rows = BUNDLE_ASSET_NAMES.map((name) => {
    const matches = fs
      .readdirSync(root, { recursive: true })
      .map((entry) => path.join(root, entry))
      .filter((entry) => path.basename(entry) === name && fs.statSync(entry).isFile());
    assert.equal(matches.length, 1, `fixture should contain one ${name}`);
    return `${sha256(matches[0])}  ${name}`;
  }).sort();
  fs.writeFileSync(
    path.join(bundleManifestDirectory, BUNDLE_CHECKSUM_MANIFEST),
    `${rows.join("\n")}\n`,
  );
}

test("authoritative release inventory contains seven targets and 34 assets", () => {
  const assets = allReleaseAssetNames();
  assert.equal(assets.length, 34);
  assert.equal(checksummedReleaseAssetNames().length, 33);
  for (const required of [
    "codewhale-android-arm64",
    "codew-android-arm64",
    "codewhale-windows-arm64.exe",
    "codew-windows-arm64.exe",
    "codewhale-windows-arm64.zip",
    "CodeWhaleSetup.exe",
  ]) {
    assert.ok(assets.includes(required), `missing ${required}`);
  }
});

test("assembly creates and verifies the exact release asset directory", async () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "codewhale-asset-assembly-"));
  const input = path.join(tempRoot, "input");
  const output = path.join(tempRoot, "output");
  try {
    fs.mkdirSync(input, { recursive: true });
    makeIntermediateArtifacts(input);
    await assemble(input, output);
    await assert.doesNotReject(() => verifyAssetDirectory(output));

    assert.deepEqual(
      fs.readdirSync(output).sort(),
      [...allReleaseAssetNames()].sort(),
    );
    assert.equal(
      fs.readFileSync(path.join(output, "codewhale.bat"), "utf8"),
      windowsLauncherContents(),
    );

    const checksums = parseChecksumManifest(
      fs.readFileSync(path.join(output, CHECKSUM_MANIFEST), "utf8"),
      CHECKSUM_MANIFEST,
    );
    assert.deepEqual([...checksums.keys()].sort(), [...checksummedReleaseAssetNames()].sort());
  } finally {
    fs.rmSync(tempRoot, { force: true, recursive: true });
  }
});

test("bundle helper creates the exact nine archives and checksum manifest", () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "codewhale-bundle-assembly-"));
  const input = path.join(tempRoot, "input");
  const output = path.join(tempRoot, "output");
  const repeatedOutput = path.join(tempRoot, "output-repeated");
  try {
    fs.mkdirSync(input, { recursive: true });
    for (const name of allReleaseAssetNames().filter((asset) =>
      /^(codewhale|codew|codewhale-tui)-(linux|android|macos|windows)-/.test(asset) &&
      !asset.endsWith(".tar.gz") &&
      !asset.endsWith(".zip"),
    )) {
      const artifactDirectory = path.join(input, name);
      fs.mkdirSync(artifactDirectory, { recursive: true });
      // GitHub's artifact transport normalizes regular files to 0644. The
      // bundler must restore executable modes for non-Windows archives.
      fs.writeFileSync(path.join(artifactDirectory, name), `fixture:${name}\n`, { mode: 0o644 });
    }

    execFileSync(
      "bash",
      [path.join(repoRoot, "scripts/release/create-release-bundles.sh"), input, output],
      { cwd: repoRoot, stdio: "pipe" },
    );
    execFileSync(
      "bash",
      [path.join(repoRoot, "scripts/release/create-release-bundles.sh"), input, repeatedOutput],
      { cwd: repoRoot, stdio: "pipe" },
    );
    assert.deepEqual(
      fs.readdirSync(output).sort(),
      [...BUNDLE_ASSET_NAMES, BUNDLE_CHECKSUM_MANIFEST].sort(),
    );
    const checksums = parseChecksumManifest(
      fs.readFileSync(path.join(output, BUNDLE_CHECKSUM_MANIFEST), "utf8"),
      BUNDLE_CHECKSUM_MANIFEST,
    );
    for (const name of BUNDLE_ASSET_NAMES) {
      assert.equal(checksums.get(name), sha256(path.join(output, name)));
      assert.deepEqual(
        fs.readFileSync(path.join(output, name)),
        fs.readFileSync(path.join(repeatedOutput, name)),
        `${name} should be byte-reproducible for identical inputs`,
      );
    }
    assert.deepEqual(
      fs.readFileSync(path.join(output, BUNDLE_CHECKSUM_MANIFEST)),
      fs.readFileSync(path.join(repeatedOutput, BUNDLE_CHECKSUM_MANIFEST)),
      "bundle checksum manifest should be reproducible",
    );

    const linuxEntries = execFileSync(
      "tar",
      ["-tzf", path.join(output, "codewhale-linux-x64.tar.gz")],
      { encoding: "utf8" },
    );
    for (const entry of ["codewhale", "codew", "codewhale-tui", "install.sh"]) {
      assert.match(linuxEntries, new RegExp(`codewhale-linux-x64/${entry}\\n`));
    }
    const extracted = path.join(tempRoot, "extracted");
    fs.mkdirSync(extracted);
    execFileSync(
      "tar",
      ["-xzf", path.join(output, "codewhale-linux-x64.tar.gz"), "-C", extracted],
      { stdio: "pipe" },
    );
    for (const entry of ["codewhale", "codew", "codewhale-tui", "install.sh"]) {
      const mode = fs.statSync(path.join(extracted, "codewhale-linux-x64", entry)).mode & 0o777;
      assert.equal(mode, 0o755, `${entry} should remain executable after artifact transport`);
    }
    const portableEntries = execFileSync(
      "unzip",
      ["-Z1", path.join(output, "codewhale-windows-arm64-portable.zip")],
      { encoding: "utf8" },
    );
    assert.match(portableEntries, /codewhale-windows-arm64-portable\/codew\.exe\n/);
    assert.doesNotMatch(portableEntries, /install\.bat/);
  } finally {
    fs.rmSync(tempRoot, { force: true, recursive: true });
  }
});

test("verification rejects modified and unexpected assets", async () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "codewhale-asset-tamper-"));
  const input = path.join(tempRoot, "input");
  const output = path.join(tempRoot, "output");
  try {
    fs.mkdirSync(input, { recursive: true });
    makeIntermediateArtifacts(input);
    await assemble(input, output);
    fs.appendFileSync(path.join(output, "codewhale-linux-x64"), "tampered\n");
    await assert.rejects(
      () => verifyAssetDirectory(output),
      /checksum mismatch for codewhale-linux-x64/,
    );

    fs.writeFileSync(path.join(output, "unexpected.txt"), "unexpected\n");
    await assert.rejects(
      () => verifyAssetDirectory(output),
      /unexpected: unexpected\.txt/,
    );
  } finally {
    fs.rmSync(tempRoot, { force: true, recursive: true });
  }
});

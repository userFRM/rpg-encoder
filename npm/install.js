#!/usr/bin/env node

const { execSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const https = require("https");

const pkg = require("./package.json");
const VERSION = `v${pkg.version}`;
const REPO = "userFRM/rpg-encoder";
const BIN_DIR = path.join(__dirname, "bin");

function getTarget() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === "darwin" && arch === "arm64") return "aarch64-apple-darwin";
  if (platform === "darwin" && arch === "x64") return "x86_64-apple-darwin";
  if (platform === "linux" && arch === "x64") return "x86_64-unknown-linux-gnu";
  if (platform === "win32" && arch === "x64") return "x86_64-pc-windows-msvc";

  throw new Error(
    `Unsupported platform: ${platform}-${arch}. ` +
      `Build from source: https://github.com/${REPO}#install`
  );
}

function download(url, dest) {
  return new Promise((resolve, reject) => {
    const follow = (u) => {
      https
        .get(u, { headers: { "User-Agent": "rpg-encoder-npm" } }, (res) => {
          if (
            res.statusCode >= 300 &&
            res.statusCode < 400 &&
            res.headers.location
          ) {
            return follow(res.headers.location);
          }
          if (res.statusCode !== 200) {
            return reject(new Error(`HTTP ${res.statusCode} for ${u}`));
          }
          const file = fs.createWriteStream(dest);
          res.pipe(file);
          file.on("finish", () => file.close(resolve));
          file.on("error", reject);
        })
        .on("error", reject);
    };
    follow(url);
  });
}

async function main() {
  const target = getTarget();
  const isWindows = process.platform === "win32";
  const ext = isWindows ? "zip" : "tar.gz";
  const archive = `rpg-encoder-${target}.${ext}`;
  const url = `https://github.com/${REPO}/releases/download/${VERSION}/${archive}`;

  fs.mkdirSync(BIN_DIR, { recursive: true });

  const tmpFile = path.join(BIN_DIR, archive);

  console.log(`Downloading rpg-encoder ${VERSION} for ${target}...`);

  try {
    await download(url, tmpFile);

    // Extract using system tar (available on macOS, Linux, and modern Windows)
    if (isWindows) {
      execSync(`tar -xf "${tmpFile}" -C "${BIN_DIR}"`, { stdio: "ignore" });
    } else {
      execSync(`tar -xzf "${tmpFile}" -C "${BIN_DIR}"`, { stdio: "ignore" });
    }
  } catch (err) {
    console.error(`Failed to download pre-built binary: ${err.message}`);
    console.error(`\nYou can build from source instead:`);
    console.error(`  git clone https://github.com/${REPO}.git`);
    console.error(`  cd rpg-encoder && cargo build --release`);
    process.exit(1);
  } finally {
    // Clean up archive
    if (fs.existsSync(tmpFile)) fs.unlinkSync(tmpFile);
  }

  // Make binaries executable
  if (!isWindows) {
    for (const bin of ["rpg-encoder", "rpg-mcp-server"]) {
      const p = path.join(BIN_DIR, bin);
      if (fs.existsSync(p)) fs.chmodSync(p, 0o755);
    }
  }

  console.log("rpg-encoder installed successfully.");
}

main();

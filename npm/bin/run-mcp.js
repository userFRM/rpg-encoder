#!/usr/bin/env node

const { execFileSync } = require("child_process");
const path = require("path");

const ext = process.platform === "win32" ? ".exe" : "";
const bin = path.join(__dirname, `rpg-mcp-server${ext}`);

try {
  execFileSync(bin, process.argv.slice(2), { stdio: "inherit" });
} catch (err) {
  if (err.status != null) process.exit(err.status);
  throw err;
}

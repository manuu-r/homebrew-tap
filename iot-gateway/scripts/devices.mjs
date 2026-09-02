#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import {
  activateSql,
  enabledSql,
  generateDeviceToken,
  hashDeviceToken,
  listSql,
  provisionSql,
  stageSql,
  validateDeviceId,
} from "./device-lib.mjs";

const DATABASE_NAME = "iot-gateway-db";
const PROJECT_ROOT = fileURLToPath(new URL("..", import.meta.url));
const args = process.argv.slice(2);
const action = args[0];
const flags = args.slice(1).filter((argument) => argument.startsWith("--"));
const positional = args.slice(1).filter((argument) => !argument.startsWith("--"));
const deviceId = positional[0];

if (!action || ["help", "--help", "-h"].includes(action)) {
  printUsage();
  process.exit(action ? 0 : 1);
}

const remote = flags.includes("--remote");
const local = flags.includes("--local");
const tokenOnly = flags.includes("--token-only");
const unknownFlags = flags.filter(
  (flag) => !["--local", "--remote", "--token-only"].includes(flag),
);
if (unknownFlags.length > 0) {
  fail(`Unknown flag${unknownFlags.length === 1 ? "" : "s"}: ${unknownFlags.join(", ")}`);
}

if (remote === local) {
  fail("Choose exactly one target: --local or --remote.");
}

let sql;
let generatedToken;

try {
  switch (action) {
    case "provision": {
      requirePositionals(positional, 1, action);
      validateDeviceId(deviceId);
      generatedToken = generateDeviceToken();
      sql = provisionSql(deviceId, hashDeviceToken(generatedToken));
      break;
    }
    case "stage": {
      requirePositionals(positional, 1, action);
      validateDeviceId(deviceId);
      generatedToken = generateDeviceToken();
      sql = stageSql(deviceId, hashDeviceToken(generatedToken));
      break;
    }
    case "activate":
      requirePositionals(positional, 1, action);
      sql = activateSql(validateDeviceId(deviceId));
      break;
    case "enable":
      requirePositionals(positional, 1, action);
      sql = enabledSql(validateDeviceId(deviceId), true);
      break;
    case "disable":
      requirePositionals(positional, 1, action);
      sql = enabledSql(validateDeviceId(deviceId), false);
      break;
    case "list":
      requirePositionals(positional, 0, action);
      sql = listSql();
      break;
    default:
      fail(`Unknown action: ${action}`);
  }
} catch (error) {
  fail(error instanceof Error ? error.message : String(error));
}

if (tokenOnly && !generatedToken) {
  fail("--token-only is valid only with provision or stage.");
}

const target = remote ? "--remote" : "--local";
const wranglerExecutable = fileURLToPath(
  new URL(
    process.platform === "win32"
      ? "node_modules/.bin/wrangler.cmd"
      : "node_modules/.bin/wrangler",
    new URL("../", import.meta.url),
  ),
);

if (!existsSync(wranglerExecutable)) {
  fail(`Wrangler is not installed. Run 'npm install' in ${PROJECT_ROOT} first.`);
}

const result = spawnSync(
  wranglerExecutable,
  ["d1", "execute", DATABASE_NAME, target, "--command", sql],
  {
    cwd: PROJECT_ROOT,
    encoding: "utf8",
    stdio: ["inherit", "pipe", "pipe"],
    shell: false,
  },
);

if (result.error) {
  fail(`Could not start Wrangler: ${result.error.message}`);
}

if (result.status !== 0) {
  if (result.stdout) process.stdout.write(result.stdout);
  if (result.stderr) process.stderr.write(result.stderr);
  process.exit(result.status || 1);
}

if (tokenOnly) {
  process.stdout.write(`${generatedToken}\n`);
} else {
  if (result.stdout) process.stdout.write(result.stdout);
  if (result.stderr) process.stderr.write(result.stderr);
}

if (generatedToken && !tokenOnly) {
  process.stdout.write(
    `\nDevice '${deviceId}' was provisioned. Save this token now; it is not stored in plaintext:\n\n${generatedToken}\n\n`,
  );
}

function printUsage() {
  process.stdout.write(`Usage:
  npm run device -- provision <device-id> --local|--remote
  npm run device -- stage     <device-id> --local|--remote [--token-only]
  npm run device -- activate  <device-id> --local|--remote
  npm run device -- disable   <device-id> --local|--remote
  npm run device -- enable    <device-id> --local|--remote
  npm run device -- list                  --local|--remote

provision rotates immediately. stage + activate provide a safe two-phase
rotation for firmware uploads; while staged, both old and new tokens work.
`);
}

function fail(message) {
  process.stderr.write(`Error: ${message}\n`);
  printUsage();
  process.exit(1);
}

function requirePositionals(values, expected, command) {
  if (values.length !== expected) {
    fail(
      expected === 0
        ? `${command} does not accept a device ID.`
        : `${command} requires exactly one device ID.`,
    );
  }
}

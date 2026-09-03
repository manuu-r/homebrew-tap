import { readFile } from "node:fs/promises";
import process from "node:process";

import WebSocket from "ws";

const [pcmPath, credentialPath] = process.argv.slice(2);
if (!pcmPath || !credentialPath) {
  console.error("usage: node scripts/live-smoke.mjs AUDIO.pcm CREDENTIAL_HEADER");
  process.exit(2);
}

const [audio, credentialHeader] = await Promise.all([
  readFile(pcmPath),
  readFile(credentialPath, "utf8"),
]);
const token = credentialHeader.match(
  /^#define IOT_GATEWAY_TOKEN "([^"]+)"$/m,
)?.[1];
if (!token) {
  console.error("No gateway token found in the credential header.");
  process.exit(2);
}

const socket = new WebSocket("wss://bunty.underdogthinkers.com/v1/live", {
  headers: {
    Authorization: `Bearer ${token}`,
    "X-Request-ID": `bunty:smoke:${Date.now()}`,
  },
});

let started = false;
let outputBytes = 0;
const timeout = setTimeout(() => {
  console.error("smoke test timed out");
  socket.close();
  process.exitCode = 1;
}, 60_000);

socket.on("message", async (data, isBinary) => {
  if (isBinary) {
    outputBytes += data.byteLength;
    return;
  }

  const message = JSON.parse(data.toString());
  const detail = message.text || message.code || message.message;
  const suffix = detail ? `: ${detail}` : "";
  console.log(`${message.type}${suffix}`);

  if (message.type === "session.ready" && !started) {
    started = true;
    // Flux performs best with real-time 80 ms PCM chunks.
    const frameBytes = 2_560;
    for (let offset = 0; offset < audio.byteLength; offset += frameBytes) {
      socket.send(audio.subarray(offset, offset + frameBytes));
      await new Promise((resolve) => setTimeout(resolve, 80));
    }
    socket.send(JSON.stringify({ type: "input.end" }));
  }

  if (message.type === "audio.end") {
    socket.send(
      JSON.stringify({ type: "playback.finished", turn: message.turn }),
    );
  }

  if (message.type === "session.complete") {
    console.log(`output.audio.bytes=${outputBytes}`);
    clearTimeout(timeout);
    socket.close();
  }
});

socket.on("error", (error) => {
  clearTimeout(timeout);
  console.error(`websocket error: ${error.message}`);
  process.exitCode = 1;
});

socket.on("close", () => {
  clearTimeout(timeout);
});

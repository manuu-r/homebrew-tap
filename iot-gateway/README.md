# IoT Gateway

A narrow Cloudflare Worker API for sending bounded IoT requests to Claude without placing an Anthropic or Cloudflare AI Gateway credential on any device.

```text
IoT device -- bearer token + PCM WebSocket --> Worker --> AI Gateway
                                                   |          |
                                                   |          +-- Flux live STT
                                                   |          +-- Claude Haiku
                                                   |          +-- Aura 2 Aries TTS
                                                   +-- D1 device registry
                                                   +-- per-device rate limits
```

The public API deliberately does **not** accept a model, system prompt, tools, provider headers, or raw Anthropic request body. The Worker chooses those values and caps request size and output tokens.

## What is included

- `POST /v1/infer` with a small JSON schema
- `GET /v1/live` WebSocket for raw PCM → Flux → Claude → Aura Aries → raw PCM
- Per-device random bearer tokens; only SHA-256 hashes are stored in D1
- Individual device enable/disable and token rotation
- Per-device Cloudflare Workers rate limiting
- AI Gateway metadata for per-device spend limits
- Anthropic BYOK through the pre-authenticated Workers AI binding
- Prompt logging disabled by default
- Local tests that require no cloud account

## 1. Install and test

Requires Node.js 20 or newer.

```sh
npm install
npm test
```

## 2. Create the AI Gateway

In the Cloudflare dashboard:

1. Open **AI > AI Gateway**.
2. Create a gateway with the ID `iot-gateway`.
3. Enable authenticated gateway access.
4. Under **Provider Keys**, add the Anthropic API key using the `default` alias.

The key is then stored by Cloudflare. It is not added to this project. If you prefer Cloudflare Unified Billing and the selected model is available there, you can omit the Anthropic key.

The Worker uses an AI binding, which Cloudflare pre-authenticates within the same account. No AI Gateway bearer token is required in the Worker.

## 3. Create D1 and apply the schema

Authenticate Wrangler, create the database, and copy the returned `database_id` into `wrangler.jsonc`:

```sh
npx wrangler login
npm run db:create
```

Replace this placeholder:

```json
"database_id": "REPLACE_WITH_YOUR_D1_DATABASE_ID"
```

Apply the migration:

```sh
npm run db:migrate:remote
```

For local development, also run:

```sh
npm run db:migrate:local
```

## 4. Review the configuration

Edit `wrangler.jsonc` before deploying:

- `AI_GATEWAY_ID`: your AI Gateway ID.
- `AI_MODEL`: a currently available Cloudflare model ID such as `anthropic/claude-haiku-4.5`.
- `STT_MODEL`: `@cf/deepgram/flux`, Cloudflare's live conversational speech-recognition model.
- `TTS_MODEL`: `@cf/deepgram/aura-2-en` with `TTS_SPEAKER` set to `aries`.
- `SYSTEM_PROMPT`: the trusted instruction applied to every text request.
- `VOICE_SYSTEM_PROMPT`: currently asks Claude for a concise, single-line voice reply.
- `AI_GATEWAY_LOGS`: `false` by default so prompts are not retained in Gateway logs.
- `LIVE_AUDIO_SAMPLE_RATE`: `16000`; both directions use signed 16-bit mono PCM.
- Live session duration, audio, frame, turn, and playback-ack limits.
- Token and text limits: global hard limits enforced in addition to each device's limits.
- `namespace_id`: change `1001` or `1002` if either rate-limit namespace is already used elsewhere in your Cloudflare account.

Cloudflare names the requested voice as a model/speaker pair rather than one
literal model ID: `aura-2-aries-en` means `@cf/deepgram/aura-2-en` plus
`speaker: "aries"`. Flux and Aura are Cloudflare-hosted Workers AI models, so
they do not require an additional Deepgram API key. Your Anthropic
key remains in AI Gateway as described above.

Each voice utterance is its own session. Bunty detects speech and the final
pause locally, opens the socket only after speech begins, and sends `input.end`
after the trailing silence. The Worker forwards `ForceEndTurn` followed by
`CloseStream` to Flux, then runs Claude and Aura and closes the device socket
after playback is acknowledged.

The configured limits allow twenty live-session openings and ten finalized
voice turns or text requests per device per 60 seconds. Cloudflare's Workers Rate
Limiting API is intentionally eventually consistent and local to a Cloudflare
location; configure an AI Gateway spend limit as the final cost backstop.

## 5. Provision a device

Create or rotate a device token:

```sh
npm run device -- provision kitchen-sensor-01 --remote
```

The token is displayed once. Store it in the device's secure storage. Running `provision` again for the same ID immediately replaces the stored hash, invalidating the old token.

List or disable devices:

```sh
npm run device -- list --remote
npm run device -- disable kitchen-sensor-01 --remote
npm run device -- enable kitchen-sensor-01 --remote
```

For local D1, substitute `--local` for `--remote`.

### How `--remote` reaches D1

The device command runs the project's local Wrangler executable as:

```sh
wrangler d1 execute iot-gateway-db --remote --command "..."
```

Wrangler reads the `database_id` from `wrangler.jsonc`, obtains your Cloudflare
account authorization from `wrangler login` (or `CLOUDFLARE_API_TOKEN` and
`CLOUDFLARE_ACCOUNT_ID`), and sends the SQL operation to Cloudflare's D1 API over HTTPS. The
Worker does not need to be running locally or already deployed. Without
`--remote`, Wrangler instead changes the emulated database under `.wrangler/`.

You can verify which Cloudflare account the local CLI will use with:

```sh
npx wrangler whoami
```

### Safe firmware rotation

When `BUNTY_PROVISION_IOT=1` is set for a Bunty PlatformIO upload, the hook uses
the internal `stage` and `activate` commands. Staging adds a pending token
without invalidating the active one; authentication accepts either hash. A
successful flash activates the pending token and retires the old one. This
prevents a failed compile or upload from locking the existing firmware out.
Ordinary uploads do not contact Cloudflare and preserve the device's existing
NVS credential.

## 6. Run and deploy

```sh
npm run dev
npm run deploy
```

Health check:

```sh
curl https://YOUR-WORKER.workers.dev/health
```

Inference request:

```sh
curl https://YOUR-WORKER.workers.dev/v1/infer \
  --request POST \
  --header "Authorization: Bearer YOUR_DEVICE_TOKEN" \
  --header "Content-Type: application/json" \
  --data '{
    "request_id": "reading:123",
    "input": "Explain whether this reading needs attention in one sentence.",
    "context": {
      "temperature_c": 48.2,
      "battery_percent": 71
    },
    "max_output_tokens": 100
  }'
```

Request fields:

| Field | Required | Constraint |
| --- | --- | --- |
| `input` | Yes | Non-empty string, capped per device |
| `context` | No | JSON object; treated as untrusted data |
| `max_output_tokens` | No | Positive integer below device and global caps |
| `request_id` | No | 1-64 safe identifier characters |

Example response:

```json
{
  "request_id": "reading:123",
  "result": {
    "content": [
      {
        "type": "text",
        "text": "The temperature is elevated and should be checked promptly."
      }
    ]
  }
}
```

### Live voice WebSocket

Open an authenticated WebSocket connection—there is no audio-file upload:

```text
wss://YOUR-WORKER.workers.dev/v1/live
Authorization: Bearer YOUR_DEVICE_TOKEN
X-Request-ID: bunty:live:123        # optional
```

The connection runs this fixed, half-duplex pipeline:

```text
binary PCM frames
  -> @cf/deepgram/flux live WebSocket
  -> Flux EndOfTurn
  -> anthropic/claude-haiku-4.5
  -> @cf/deepgram/aura-2-en (speaker: aries, linear16)
  -> binary PCM frames on the same device WebSocket
```

Input frames must be raw signed little-endian 16-bit mono PCM at 16 kHz. Do not
add a WAV header and do not base64-encode the data. Send 80 ms frames (2,560
bytes at this format), which is Flux's recommended chunk size. The gateway
forwards frames as they arrive rather than retaining a complete recording.

Client-to-gateway messages:

| Message | Format | Purpose |
| --- | --- | --- |
| Microphone audio | Binary | Raw `linear16` PCM frames |
| `ping` | `{"type":"ping"}` | Application heartbeat; receives `pong` |
| Playback complete | `{"type":"playback.finished","turn":"3"}` | Confirms the I2S output buffer has fully drained |
| Close | `{"type":"session.close"}` | Gracefully ends the session |

Gateway-to-client JSON controls:

| Type | Device behavior |
| --- | --- |
| `session.ready` | Verify the negotiated input/output audio format |
| `transcript.partial` / `transcript.final` | Optional UI or serial diagnostics |
| `input.pause` | Stop sending microphone data before speaker playback |
| `assistant.thinking` / `assistant.text` | Optional status and diagnostics |
| `audio.start` | Prepare the I2S output path for raw PCM |
| Binary messages | Buffer and play the returned `linear16` PCM |
| `audio.end` | Drain the playback buffer, then send `playback.finished` |
| `input.resume` | Resume microphone streaming |
| `error` | Inspect `code`; apply `retry_after` when present |

The playback acknowledgment is important: the gateway keeps microphone input
paused after `audio.end` until the device confirms its speaker buffer is empty.
This prevents Bunty from transcribing its own voice. A 30-second fallback
resumes input if the acknowledgment is lost.

Each finalized turn is currently independent; conversation history is not sent
to Claude. The gateway does not write PCM, transcripts, or model responses to
application logs. A session uses streaming STT plus one Claude and one TTS call
per finalized turn, so include all three stages in spend limits.

## 7. Attach a custom domain

After the first deployment, open **Workers & Pages > iot-gateway > Settings > Domains & Routes**, choose **Add > Custom Domain**, and enter a hostname such as `iot-api.example.com`.

Keep `workers.dev` enabled until the custom hostname is verified. You can then disable the public `workers.dev` route if the custom domain should be the only endpoint.

Alternatively, manage the Custom Domain in `wrangler.jsonc`:

```jsonc
"routes": [
  {
    "pattern": "iot-api.example.com",
    "custom_domain": true
  }
]
```

Use the exact hostname without `/*`. The hostname must belong to an active zone in the same Cloudflare account.

## 8. Configure cost protection

In AI Gateway, create:

1. A global spend limit for the entire gateway.
2. A spend limit split by the metadata key `device_id`.
3. Optional timeout, retry, or cheaper-model fallback rules.

The Worker sets `device_id` itself after authentication, so devices cannot claim another device's budget bucket.

## Production notes

- Use one token per physical device. Never ship the same credential across a fleet.
- Tokens are bearer credentials and therefore must only be sent over HTTPS.
- A provisioning token is printed to the terminal once; do not put it in source control or firmware repositories.
- D1 provides quick revocation. For higher-assurance hardware, replace bearer authentication with signed requests or mTLS backed by a secure element.
- The Worker does not add CORS headers because IoT clients do not need browser access.
- Logs intentionally contain request IDs and device IDs, but not tokens, inputs, contexts, or model responses.
- Live PCM, transcripts, and spoken replies are not written to application logs; keep `AI_GATEWAY_LOGS` disabled if they must also stay out of Gateway logs.

## Cloudflare references

- [AI Gateway Worker bindings](https://developers.cloudflare.com/ai-gateway/usage/worker-binding-methods/)
- [AI Gateway BYOK provider keys](https://developers.cloudflare.com/ai-gateway/configuration/bring-your-own-keys/)
- [Claude Haiku model input](https://developers.cloudflare.com/ai/models/anthropic/claude-haiku-4.5/)
- [AI Gateway realtime WebSockets](https://developers.cloudflare.com/ai-gateway/usage/websockets-api/realtime-api/)
- [Deepgram Flux live STT](https://developers.cloudflare.com/workers-ai/models/flux/)
- [Aura 2 English and Aries speaker](https://developers.cloudflare.com/workers-ai/models/aura-2-en/)
- [D1 migrations](https://developers.cloudflare.com/d1/reference/migrations/)
- [Workers Rate Limiting binding](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/)
- [Workers Custom Domains](https://developers.cloudflare.com/workers/configuration/routing/custom-domains/)

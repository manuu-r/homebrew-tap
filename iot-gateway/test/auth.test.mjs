import assert from "node:assert/strict";
import test from "node:test";

import {
  authenticateDevice,
  parseBearerToken,
  sha256Hex,
} from "../src/auth.js";
import {
  activateSql,
  hashDeviceToken,
  stageSql,
} from "../scripts/device-lib.mjs";

test("parseBearerToken accepts only the provisioned token shape", () => {
  assert.equal(parseBearerToken("Bearer iot_abc-123_XYZ"), "iot_abc-123_XYZ");
  assert.equal(parseBearerToken("bearer iot_abc"), null);
  assert.equal(parseBearerToken("Bearer other_abc"), null);
  assert.equal(parseBearerToken("Bearer iot_abc extra"), null);
  assert.equal(parseBearerToken(null), null);
});

test("Worker and provisioning script calculate identical token hashes", async () => {
  const token = "iot_a_secure_test_token";
  assert.equal(await sha256Hex(token), hashDeviceToken(token));
});

test("authentication accepts the active or upload-staged token column", async () => {
  let lookupSql = "";
  const database = {
    prepare(sql) {
      lookupSql = sql;
      return {
        bind() {
          return {
            async first() {
              return {
                id: "bunty",
                enabled: 1,
                max_input_chars: 2000,
                max_output_tokens: 256,
              };
            },
          };
        },
      };
    },
  };

  const device = await authenticateDevice(
    new Request("https://iot.example/v1/infer", {
      headers: { Authorization: "Bearer iot_staged_token" },
    }),
    database,
  );

  assert.equal(device.id, "bunty");
  assert.match(lookupSql, /token_hash = \?1 OR pending_token_hash = \?1/);
});

test("flash rotation SQL stages and then promotes a token", () => {
  const hash = "a".repeat(64);
  assert.match(stageSql("bunty", hash), /pending_token_hash/);
  assert.match(stageSql("bunty", hash), /ON CONFLICT\(id\) DO UPDATE/);
  assert.match(activateSql("bunty"), /token_hash = pending_token_hash/);
  assert.match(activateSql("bunty"), /pending_token_hash = NULL/);
});

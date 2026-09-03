import test from "node:test";
import assert from "node:assert/strict";

import {
  appendTurn,
  loadHistory,
  normalizeSessionId,
} from "../src/history.js";

// Minimal D1 stand-in: records prepared SQL and its bound args, returns
// whatever the test queued for the next .all().
function makeDb() {
  const calls = [];
  let nextRows = [];
  const db = {
    calls,
    queue(rows) {
      nextRows = rows;
    },
    prepare(sql) {
      const record = { sql, args: [] };
      return {
        bind(...args) {
          record.args = args;
          return this;
        },
        async run() {
          calls.push({ ...record, kind: "run" });
          return { success: true };
        },
        async all() {
          calls.push({ ...record, kind: "all" });
          const rows = nextRows;
          nextRows = [];
          return { results: rows };
        },
      };
    },
    async batch(statements) {
      for (const statement of statements) await statement.run();
      return statements.map(() => ({ success: true }));
    },
  };
  return db;
}

test("normalizeSessionId accepts a boot nonce and rejects junk", () => {
  assert.equal(normalizeSessionId("a1b2c3d4e5f6a7b8"), "a1b2c3d4e5f6a7b8");
  assert.equal(normalizeSessionId("short"), null);
  assert.equal(normalizeSessionId("has-dashes-in-it-xxxx"), null);
  assert.equal(normalizeSessionId(null), null);
  assert.equal(normalizeSessionId(undefined), null);
});

test("loadHistory returns chronological user/assistant messages", async () => {
  const db = makeDb();
  // Stored newest-first as the query orders it.
  db.queue([
    { turn_index: 2, role: "assistant", content: "reply two" },
    { turn_index: 2, role: "user", content: "and now" },
    { turn_index: 1, role: "assistant", content: "reply one" },
    { turn_index: 1, role: "user", content: "first" },
  ]);

  const messages = await loadHistory({ DEVICES_DB: db }, "bunty", "sess", 6);

  assert.deepEqual(messages, [
    { role: "user", content: "first" },
    { role: "assistant", content: "reply one" },
    { role: "user", content: "and now" },
    { role: "assistant", content: "reply two" },
  ]);
  assert.equal(db.calls[0].args[2], 12, "asks D1 for maxTurns * 2 rows");
});

test("loadHistory drops an incomplete turn left by the row limit", async () => {
  const db = makeDb();
  db.queue([
    { turn_index: 3, role: "assistant", content: "kept reply" },
    { turn_index: 3, role: "user", content: "kept" },
    // Turn 2's assistant row fell outside the LIMIT window.
    { turn_index: 2, role: "user", content: "orphan question" },
  ]);

  const messages = await loadHistory({ DEVICES_DB: db }, "bunty", "sess", 1);

  // Only whole user+assistant pairs survive, so roles still strictly alternate.
  assert.deepEqual(messages, [
    { role: "user", content: "kept" },
    { role: "assistant", content: "kept reply" },
  ]);
});

test("loadHistory is inert without a database or session", async () => {
  assert.deepEqual(await loadHistory({}, "bunty", "sess", 6), []);
  assert.deepEqual(
    await loadHistory({ DEVICES_DB: makeDb() }, "bunty", null, 6),
    [],
  );
  assert.deepEqual(
    await loadHistory({ DEVICES_DB: makeDb() }, "bunty", "sess", 0),
    [],
  );
});

test("loadHistory swallows a D1 failure", async () => {
  const db = {
    prepare() {
      return {
        bind() {
          return this;
        },
        async all() {
          throw new Error("D1 unavailable");
        },
      };
    },
  };
  assert.deepEqual(await loadHistory({ DEVICES_DB: db }, "bunty", "sess", 6), []);
});

test("appendTurn writes both roles and prunes older turns", async () => {
  const db = makeDb();
  await appendTurn(
    { DEVICES_DB: db },
    "bunty",
    "sess",
    4,
    "what time is it",
    "time you got a clock",
    6,
  );

  const runs = db.calls.filter((call) => call.kind === "run");
  assert.equal(runs.length, 3);
  assert.equal(runs[0].args[3], "what time is it");
  assert.equal(runs[0].sql.includes("'user'"), true);
  assert.equal(runs[1].args[3], "time you got a clock");
  assert.equal(runs[1].sql.includes("'assistant'"), true);
  assert.equal(runs[2].sql.includes("DELETE"), true);
  assert.equal(runs[2].args[2], 6, "keeps the configured number of turns");
});

test("appendTurn skips a turn missing either side", async () => {
  const db = makeDb();
  await appendTurn({ DEVICES_DB: db }, "bunty", "sess", 1, "hi", "", 6);
  await appendTurn({ DEVICES_DB: db }, "bunty", "sess", 1, "", "hello", 6);
  assert.equal(db.calls.length, 0);
});

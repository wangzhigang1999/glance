import { test } from "node:test";
import assert from "node:assert/strict";
import {
  configPatch,
  logLevel,
} from "../src/lib/policy.mjs";
test("settings patch preserves masked secrets and unrelated configuration", () => {
  const saved = {
    gh_token: "***abc",
    gh_user: "alice",
    market_token: "private",
    sensor_refresh_s: 5,
  };
  assert.deepEqual(configPatch(saved, { ...saved, sensor_refresh_s: "10" }), {
    sensor_refresh_s: 10,
  });
  assert.deepEqual(configPatch(saved, { ...saved, gh_token: "***other" }), {});
  assert.deepEqual(configPatch(saved, { ...saved, gh_token: "" }), {
    gh_token: "",
  });
  assert.throws(() => configPatch(saved, { ...saved, sensor_refresh_s: "" }));
});
test("log severity supports board and synthetic diagnostic lines", () => {
  assert.equal(logLevel("  128 W wifi: disconnected"), "W");
  assert.equal(logLevel("E (128) ota: rejected"), "E");
  assert.equal(logLevel("W logs: overwritten"), "W");
});

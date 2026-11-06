import assert from "node:assert/strict";
import { after, afterEach, before, beforeEach, mock, test } from "node:test";
import { fileURLToPath } from "node:url";
import { createServer, normalizePath } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const root = fileURLToPath(new URL("../", import.meta.url));
const bridgePath = normalizePath(fileURLToPath(new URL("../src/tauri.ts", import.meta.url)));
const bridgeKey = Symbol.for("cec-support.forget-test-bridge");
let server;
let store;
let bridge;

before(async () => {
  // Vite compiles the actual Svelte rune store. Override only the backend
  // boundary; no application method is copied or reimplemented in this test.
  const virtualBridge = "\0cec-forget-test-bridge";
  server = await createServer({
    root,
    configFile: false,
    logLevel: "silent",
    plugins: [
      svelte({ configFile: false, compilerOptions: { runes: true } }),
      {
        name: "forget-test-bridge",
        enforce: "pre",
        resolveId(source, importer) {
          if (source === "./tauri" && importer?.endsWith("/src/store.svelte.ts")) {
            return virtualBridge;
          }
        },
        load(id) {
          if (id !== virtualBridge) return;
          return `
            export * from ${JSON.stringify(bridgePath)};
            const bridge = () => globalThis[Symbol.for("cec-support.forget-test-bridge")];
            export const isTauri = () => true;
            export const cecRevoke = (tech) => bridge().revoke(tech);
            export const cecForgetNode = (tech) => bridge().forget(tech);
            export const cecDeny = (...args) => bridge().deny(...args);
            export const cecGrants = () => bridge().grants();
          `;
        },
      },
    ],
    server: { middlewareMode: true, watch: null },
  });
  ({ store } = await server.ssrLoadModule("/src/store.svelte.ts"));
});

beforeEach(() => {
  bridge = {
    revoke: mock.fn(async () => {}),
    forget: mock.fn(async () => {}),
    deny: mock.fn(async () => {}),
    grants: mock.fn(async () => []),
  };
  globalThis[bridgeKey] = bridge;
  store.sessions = {
    active: {
      session_id: "active",
      state: "active",
      tech: "technician-AB12C",
      agent_name: "CEC technician",
      want_control: true,
    },
  };
  store.pending = [
    ["pending-canonical", "technician"],
    ["pending-display", "technician-XY987"],
    ["other-request", "another-QQ123"],
  ].map(([session_id, tech]) => ({
    session_id,
    tech,
    agent_name: "CEC technician",
    want_control: true,
    verification_code: "123456",
  }));
  for (const request of store.pending) {
    store.sessions[request.session_id] = { ...request, state: "requested" };
  }
  store.grants = [];
  store.activeChatPeer = "technician";
  store.toast = null;
});

afterEach(() => {
  store.destroy();
  delete globalThis[bridgeKey];
});

after(async () => {
  await server?.close();
});

function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

test("forget waits for revocation and node removal before clearing an active session", async () => {
  const revoked = deferred();
  const forgotten = deferred();
  bridge.revoke.mock.mockImplementation(() => revoked.promise);
  bridge.forget.mock.mockImplementation(() => forgotten.promise);

  const removing = store.forget("technician");
  assert.deepEqual(bridge.revoke.mock.calls.map(({ arguments: args }) => args), [["technician"]]);
  assert.equal(bridge.forget.mock.callCount(), 0);
  assert.equal(bridge.deny.mock.callCount(), 0);
  assert.equal(store.liveSessions.length, 1);
  assert.equal(store.pending.length, 3);
  assert.equal(store.toast, null);

  revoked.resolve();
  await Promise.resolve();
  assert.deepEqual(bridge.forget.mock.calls.map(({ arguments: args }) => args), [["technician"]]);
  assert.equal(store.liveSessions.length, 1);
  assert.equal(store.pending.length, 3);
  assert.equal(store.activeChatPeer, "technician");
  assert.equal(store.toast, null);

  forgotten.resolve();
  await removing;
  assert.equal(bridge.deny.mock.callCount(), 0);
  assert.equal(bridge.grants.mock.callCount(), 1);
  assert.deepEqual(Object.keys(store.sessions), ["other-request"]);
  assert.equal(store.sessions["other-request"].tech, "another-QQ123");
  assert.deepEqual(store.pending.map(({ session_id }) => session_id), ["other-request"]);
  assert.equal(store.pending[0].tech, "another-QQ123");
  assert.equal(store.activeChatPeer, null);
  assert.match(store.toast, /^Disconnected and removed\./);
});

test("a rejected revoke keeps live state and reports failure without forgetting the node", async () => {
  bridge.revoke.mock.mockImplementation(async () => { throw new Error("revocation refused"); });

  await store.forget("technician");

  assert.equal(bridge.forget.mock.callCount(), 0);
  assert.equal(bridge.deny.mock.callCount(), 0);
  assert.equal(bridge.grants.mock.callCount(), 1);
  assert.equal(store.liveSessions[0].session_id, "active");
  assert.deepEqual(store.pending.map(({ session_id }) => session_id), [
    "pending-canonical", "pending-display", "other-request",
  ]);
  assert.equal(Object.keys(store.sessions).length, 4);
  assert.equal(store.activeChatPeer, "technician");
  assert.equal(store.toast, "Couldn't remove: revocation refused");
});

test("a rejected node removal keeps live state and its failure message", async () => {
  bridge.forget.mock.mockImplementation(async () => { throw new Error("node removal refused"); });

  await store.forget("technician");

  assert.equal(bridge.revoke.mock.callCount(), 1);
  assert.equal(bridge.forget.mock.callCount(), 1);
  assert.equal(bridge.deny.mock.callCount(), 0);
  assert.equal(bridge.grants.mock.callCount(), 1);
  assert.equal(store.liveSessions[0].session_id, "active");
  assert.deepEqual(store.pending.map(({ session_id }) => session_id), [
    "pending-canonical", "pending-display", "other-request",
  ]);
  assert.equal(Object.keys(store.sessions).length, 4);
  assert.equal(store.activeChatPeer, "technician");
  assert.equal(store.toast, "Couldn't remove: node removal refused");
});

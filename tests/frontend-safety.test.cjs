const fs = require('node:fs');
const path = require('node:path');
const { createRequire } = require('node:module');
const { pathToFileURL } = require('node:url');
const assert = require('node:assert/strict');
const { describe, it, before } = require('node:test');

const repo = process.argv.includes('--repo')
  ? process.argv[process.argv.indexOf('--repo') + 1]
  : path.resolve(__dirname, '..');

const req = createRequire(path.join(repo, 'package.json'));
const ts = req('typescript');
const vue = req('vue');

const console = { log() {}, error() {} };

function readFile(file) {
  return fs.readFileSync(path.join(repo, file), 'utf8');
}

function script(text) {
  const m = text.match(/<script[^>]*>([\s\S]*?)<\/script>/);
  return m ? m[1] : text;
}

function transpile(text) {
  return ts.transpileModule(text, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
}

// Intentionally compiles reviewed, pinned repository sources at runtime for deterministic
// regression tests. The evaluated code comes from the local files under test, not untrusted input.
function functions(text, names, scope) {
  const ast = ts.createSourceFile('probe.ts', script(text), ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const found = new Map();
  function visit(node) {
    if (ts.isFunctionDeclaration(node) && node.name && names.includes(node.name.text)) {
      found.set(node.name.text, node.getText(ast));
    }
    ts.forEachChild(node, visit);
  }
  visit(ast);
  for (const n of names) assert(found.has(n), `missing source function ${n}`);
  const body = [...found.values()].join('\n') + `\nreturn { ${names.join(', ')} };`;
  return new Function(...Object.keys(scope), transpile(body))(...Object.values(scope));
}

// Intentionally compiles reviewed, pinned repository sources at runtime for deterministic
// regression tests. The evaluated code comes from the local files under test, not untrusted input.
function moduleFrom(text, modules) {
  const exports = {};
  new Function('require', 'exports', transpile(text))((name) => {
    if (!(name in modules)) throw new Error(`unmocked import ${name}`);
    return modules[name];
  }, exports);
  return exports;
}

describe('Frontend safety regression', () => {
  let vueuse;

  before(async () => {
    vueuse = await import(pathToFileURL(req.resolve('@vueuse/core')).href);
  });

  describe('app-state log reactivity', () => {
    it('addLog triggers Vue watchers', async () => {
      const state = moduleFrom(readFile('src/composables/app-state.ts'), {
        vue,
        '@vueuse/core': vueuse,
      }).useGlobalState();

      let renders = 0;
      const stop = vue.watchEffect(() => {
        state.logs.value.length;
        renders++;
      });

      state.addLog('audit observation');
      await vue.nextTick();

      assert.equal(state.logs.value.length, 1);
      assert.equal(renders, 2, 'shallowRef push must notify reactive consumers');
      stop();
    });
  });

  describe('fetch-gamelist validation and fallback', () => {
    const good = { id: '123', name: 'Audit Fixture', aliases: [], executables: [] };

    function fetching(invoke, bundledStore = { default: [{ ...good, id: '456', name: 'Bundled Fixture' }] }) {
      const dialogs = [];
      const exported = moduleFrom(readFile('src/composables/fetch-gamelist.ts'), {
        vue,
        '@vueuse/core': { ...vueuse, tryOnMounted() {} },
        '@tauri-apps/api/core': { invoke },
        '@tauri-apps/plugin-dialog': {
          message: async (text) => dialogs.push(text),
        },
        '@tauri-apps/plugin-http': {},
        './app-state': { useGlobalState: () => ({ addLog() {} }) },
        '../assets/gamelist.json': bundledStore,
        '@/types/types': {},
      });
      return { api: exported.useFetchGameList(), dialogs };
    }

    it('validates every row, not only the first', () => {
      const { isValidGameList } = functions(
        readFile('src/composables/fetch-gamelist.ts'),
        ['isValidExecutable', 'isValidGame', 'isValidGameList'],
        {}
      );

      assert.equal(isValidGameList([good]), true);
      assert.equal(isValidGameList([good, { id: 'bad', name: 123, aliases: 'nope' }]), false);
      assert.equal(isValidGameList([good, null]), false);
      assert.equal(isValidGameList([]), true);
    });

    it('advances fallback when mirror returns invalid data', async () => {
      const calls = [];
      const ctx = fetching(async (command) => {
        calls.push(command);
        if (command === 'fetch_gamelist_gh_mirror') {
          return [{ ...good, name: 'Mirror Invalid', executables: 'not-array' }];
        }
        return [good];
      });

      await ctx.api.fetchGameList();

      assert.equal(calls.includes('fetch_gamelist_from_discord'), true, 'invalid mirror must fall back to Discord');
      assert.equal(ctx.api.gameDB.value[0].name, good.name);
    });

    it('advances fallback when Discord returns invalid data', async () => {
      const calls = [];
      const ctx = fetching(async (command) => {
        calls.push(command);
        if (command === 'fetch_gamelist_from_discord') {
          return [{ ...good, name: 'Discord Invalid', executables: null }];
        }
        return [{ ...good, name: 'Mirror Invalid', executables: 'not-array' }];
      });

      await ctx.api.fetchGameList();

      assert.equal(calls.includes('fetch_gamelist_gh_mirror'), true);
      assert.equal(calls.includes('fetch_gamelist_from_discord'), true);
      assert.equal(ctx.api.gameDB.value[0].name, 'Bundled Fixture');
    });

    it('clears stale fetchError on a new attempt', async () => {
      let fail = true;
      const bundledStore = { default: undefined };
      const ctx = fetching(async () => {
        if (fail) throw new Error('fixture outage');
        return [good];
      }, bundledStore);

      await ctx.api.fetchGameList();
      assert.ok(ctx.api.fetchError.value, 'first failed attempt should surface an error');

      fail = false;
      bundledStore.default = [{ ...good, id: '456', name: 'Bundled Fixture' }];
      await ctx.api.fetchGameList();
      assert.equal(ctx.api.fetchError.value, null, 'subsequent successful attempt must clear stale error');
      assert.equal(ctx.api.gameDB.value[0].name, good.name);
    });

    it('returns empty valid state and surfaces error when every source fails', async () => {
      const ctx = fetching(async () => {
        throw new Error('all sources down');
      }, { default: undefined });

      await ctx.api.fetchGameList();

      assert.ok(ctx.api.fetchError.value);
      assert.deepEqual(ctx.api.gameDB.value, [], 'final catalog must be empty when all sources fail');
    });
  });

  describe('GameExecutables path validation', () => {
    it('rejects escaping paths but preserves safe relative subdirectories', () => {
      const { isValidExecutableName } = functions(
        readFile('src/components/GameExecutables.vue'),
        ['isValidExecutableName'],
        {}
      );

      assert.equal(isValidExecutableName('../audit.bin'), false);
      assert.equal(isValidExecutableName('dir/audit.bin'), true);
      assert.equal(isValidExecutableName('dir\\audit.bin'), true);
      assert.equal(isValidExecutableName('dir/../audit.bin'), false);
      assert.equal(isValidExecutableName('/dir/audit.bin'), false);
      assert.equal(isValidExecutableName('CON.exe'), false);
      assert.equal(isValidExecutableName('audit.exe.'), false);
      assert.equal(isValidExecutableName('audit<bad>.bin'), false);
      assert.equal(isValidExecutableName('audit-only.bin'), true);
      assert.equal(isValidExecutableName('valid.exe'), true);
      assert.equal(isValidExecutableName(''), false);
      assert.equal(isValidExecutableName('.'), false);
      assert.equal(isValidExecutableName('..'), false);
    });
  });

  describe('HomeView play/stop', () => {
    function makeGame(overrides = {}) {
      return {
        id: '1',
        uid: 'g1',
        name: 'G',
        is_running: false,
        executables: [
          {
            name: 'e.exe',
            filename: 'e.exe',
            path: '',
            segments: 1,
            is_running: false,
          },
        ],
        ...overrides,
      };
    }

    it('only marks running after invoke succeeds and stores process_id', async () => {
      const game = makeGame();
      const gameList = vue.ref([game]);
      const currentlyPlaying = vue.ref(null);
      const invoke = async () => 12345;
      const scope = {
        gameList,
        currentlyPlaying,
        addLog() {},
        console,
        path: { join: async () => 'p/e.exe' },
        invoke,
      };
      const { playGame } = functions(readFile('src/pages/HomeView.vue'), ['playGame'], scope);

      await playGame({ game: gameList.value[0], executable: gameList.value[0].executables[0] });

      assert.equal(gameList.value[0].is_running, true);
      assert.equal(gameList.value[0].executables[0].is_running, true);
      assert.equal(gameList.value[0].executables[0].process_id, 12345);
      assert.equal(currentlyPlaying.value, '1');
    });

    it('preserves stopped state on launch failure', async () => {
      const game = makeGame();
      const gameList = vue.ref([game]);
      const currentlyPlaying = vue.ref(null);
      const invoke = async () => {
        throw new Error('fixture failure');
      };
      const scope = {
        gameList,
        currentlyPlaying,
        addLog() {},
        console,
        path: { join: async () => 'p/e.exe' },
        invoke,
      };
      const { playGame } = functions(readFile('src/pages/HomeView.vue'), ['playGame'], scope);

      await playGame({ game: gameList.value[0], executable: gameList.value[0].executables[0] });

      assert.equal(gameList.value[0].is_running, false);
      assert.equal(gameList.value[0].executables[0].is_running, false);
      assert.equal(currentlyPlaying.value, null);
    });

    it('stops using process_id and clears state only on success', async () => {
      const game = makeGame({
        is_running: true,
        executables: [{ name: 'e.exe', filename: 'e.exe', path: '', segments: 1, is_running: true, process_id: 12345 }],
      });
      const gameList = vue.ref([game]);
      const currentlyPlaying = vue.ref('1');
      let stoppedId = null;
      const invoke = async (cmd, payload) => {
        if (cmd === 'stop_process') stoppedId = payload.process_id;
        return;
      };
      const scope = { gameList, currentlyPlaying, addLog() {}, console, invoke };
      const { stopPlaying } = functions(readFile('src/pages/HomeView.vue'), ['stopPlaying'], scope);

      await stopPlaying({ game: gameList.value[0], executable: gameList.value[0].executables[0] });

      assert.equal(stoppedId, 12345, 'stop_process must receive the stored process_id');
      assert.equal(gameList.value[0].is_running, false);
      assert.equal(gameList.value[0].executables[0].is_running, false);
      assert.equal(currentlyPlaying.value, null);
    });

    it('keeps running state on stop failure', async () => {
      const game = makeGame({
        is_running: true,
        executables: [{ name: 'e.exe', filename: 'e.exe', path: '', segments: 1, is_running: true, process_id: 12345 }],
      });
      const gameList = vue.ref([game]);
      const currentlyPlaying = vue.ref('1');
      const invoke = async () => {
        throw new Error('stop failed');
      };
      const scope = { gameList, currentlyPlaying, addLog() {}, console, invoke };
      const { stopPlaying } = functions(readFile('src/pages/HomeView.vue'), ['stopPlaying'], scope);

      await stopPlaying({ game: gameList.value[0], executable: gameList.value[0].executables[0] });

      assert.equal(gameList.value[0].is_running, true);
      assert.equal(gameList.value[0].executables[0].is_running, true);
      assert.equal(currentlyPlaying.value, '1');
    });
  });

  describe('HomeView RPC', () => {
    function makeGame() {
      return { id: '1', uid: 'g1', name: 'G', is_running: false, executables: [] };
    }

    it('only sets connected after connect resolves successfully', async () => {
      const game = makeGame();
      const gameList = vue.ref([game]);
      const isConnecting = vue.ref(false);
      const isConnectedToRPC = vue.ref(false);
      const rpcGameId = vue.ref(null);
      const currentlyPlaying = vue.ref(null);
      const invoke = async () => undefined;
      const scope = {
        gameList,
        isConnecting,
        isConnectedToRPC,
        rpcGameId,
        currentlyPlaying,
        invoke,
        console,
        addLog() {},
        hideDialog() {},
      };
      const { continueRPCRisk } = functions(readFile('src/pages/HomeView.vue'), ['continueRPCRisk'], scope);

      await continueRPCRisk(gameList.value[0]);

      assert.equal(isConnectedToRPC.value, true);
      assert.equal(gameList.value[0].is_running, true);
      assert.equal(currentlyPlaying.value, '1');
      assert.equal(isConnecting.value, false);
    });

    it('does not set connected on connect failure', async () => {
      const game = makeGame();
      const gameList = vue.ref([game]);
      const isConnecting = vue.ref(false);
      const isConnectedToRPC = vue.ref(false);
      const rpcGameId = vue.ref(null);
      const currentlyPlaying = vue.ref(null);
      const invoke = async () => {
        throw new Error('rpc fail');
      };
      const scope = {
        gameList,
        isConnecting,
        isConnectedToRPC,
        rpcGameId,
        currentlyPlaying,
        invoke,
        console,
        addLog() {},
        hideDialog() {},
      };
      const { continueRPCRisk } = functions(readFile('src/pages/HomeView.vue'), ['continueRPCRisk'], scope);

      await continueRPCRisk(gameList.value[0]);

      assert.equal(isConnectedToRPC.value, false);
      assert.equal(gameList.value[0].is_running, false);
      assert.equal(currentlyPlaying.value, null);
      assert.equal(isConnecting.value, false);
    });

    it('disconnect awaits command and clears state only on success', async () => {
      const game = makeGame();
      game.is_running = true;
      const gameList = vue.ref([game]);
      const isConnecting = vue.ref(false);
      const isConnectedToRPC = vue.ref(true);
      const rpcGameId = vue.ref('1');
      const currentlyPlaying = vue.ref('1');
      let disconnectCalled = false;
      const invoke = async (cmd) => {
        if (cmd === 'disconnect_from_discord_rpc') disconnectCalled = true;
        return;
      };
      const scope = {
        gameList,
        isConnecting,
        isConnectedToRPC,
        rpcGameId,
        currentlyPlaying,
        invoke,
        console,
        addLog() {},
        showDialog() {},
      };
      const { handleTestRPC } = functions(readFile('src/pages/HomeView.vue'), ['handleTestRPC'], { ...scope, emit() {} });

      await handleTestRPC(gameList.value[0]);

      assert.equal(disconnectCalled, true, 'disconnect must invoke disconnect_from_discord_rpc');
      assert.equal(isConnectedToRPC.value, false);
      assert.equal(gameList.value[0].is_running, false);
      assert.equal(currentlyPlaying.value, null);
    });

    it('keeps RPC state on disconnect failure', async () => {
      const game = makeGame();
      game.is_running = true;
      const gameList = vue.ref([game]);
      const isConnecting = vue.ref(false);
      const isConnectedToRPC = vue.ref(true);
      const rpcGameId = vue.ref('1');
      const currentlyPlaying = vue.ref('1');
      const invoke = async () => {
        throw new Error('disconnect fail');
      };
      const scope = {
        gameList,
        isConnecting,
        isConnectedToRPC,
        rpcGameId,
        currentlyPlaying,
        invoke,
        console,
        addLog() {},
        showDialog() {},
      };
      const { handleTestRPC } = functions(readFile('src/pages/HomeView.vue'), ['handleTestRPC'], { ...scope, emit() {} });

      await handleTestRPC(gameList.value[0]);

      assert.equal(isConnectedToRPC.value, true);
      assert.equal(gameList.value[0].is_running, true);
      assert.equal(currentlyPlaying.value, '1');
    });
  });

  describe('Owned-process reconciliation', () => {
    function reconcile(invoke, rpcActive = false) {
      const gameList = vue.ref([{ id: '1', is_running: true, executables: [{ is_running: true, process_id: 123 }] }]);
      const scope = {
        gameList, invoke, console, currentlyPlaying: vue.ref('1'),
        isConnectedToRPC: vue.ref(rpcActive), rpcGameId: vue.ref(rpcActive ? '1' : null),
      };
      return { ...scope, ...functions(readFile('src/pages/HomeView.vue'), ['reconcileRunningProcesses'], scope) };
    }
    it('clears exited owned children and their stale process IDs', async () => {
      const ctx = reconcile(async () => []);
      await ctx.reconcileRunningProcesses();
      assert.equal(ctx.gameList.value[0].is_running, false);
      assert.equal(ctx.gameList.value[0].executables[0].process_id, undefined);
      assert.equal(ctx.currentlyPlaying.value, null);
    });
    it('preserves tracked state if the native observation fails', async () => {
      const ctx = reconcile(async () => { throw new Error('observation failed'); });
      await ctx.reconcileRunningProcesses();
      assert.equal(ctx.gameList.value[0].is_running, true);
      assert.equal(ctx.gameList.value[0].executables[0].process_id, 123);
    });
    it('does not confuse a confirmed RPC activity with an exited process', async () => {
      const ctx = reconcile(async () => [], true);
      await ctx.reconcileRunningProcesses();
      assert.equal(ctx.gameList.value[0].is_running, true);
      assert.equal(ctx.currentlyPlaying.value, '1');
      assert.equal(ctx.gameList.value[0].executables[0].is_running, false);
    });
  });

  describe('Playground read-only diagnostic', () => {
    function diagnostic(invoke) {
      const scope = {
        ipcAvailable: vue.ref(null), isChecking: vue.ref(false),
        invoke, addLog() {}, console,
      };
      return { ...scope, ...functions(readFile('src/pages/Playground.vue'), ['discordTest'], scope) };
    }

    it('checks IPC availability without publishing or disconnecting activity', async () => {
      const calls = [];
      const ctx = diagnostic(async (command) => { calls.push(command); return true; });
      await ctx.discordTest();
      await ctx.discordTest();
      assert.deepEqual(calls, ['check_discord_connection', 'check_discord_connection']);
      assert.equal(ctx.ipcAvailable.value, true);
      assert.equal(ctx.isChecking.value, false);
    });

    it('reports unavailable separately from a failed check', async () => {
      const absent = diagnostic(async () => false);
      await absent.discordTest();
      assert.equal(absent.ipcAvailable.value, false);
      const failed = diagnostic(async () => { throw new Error('probe failed'); });
      await failed.discordTest();
      assert.equal(failed.ipcAvailable.value, null);
      assert.equal(failed.isChecking.value, false);
    });

    it('has no bundled publisher, application ID or falsified timestamp helper', () => {
      const source = readFile('src/pages/Playground.vue');
      assert(!source.includes('connect_to_discord_rpc_3'));
      assert(!source.includes('1361728268088381706'));
      assert(!source.includes('createAgoTimestamp'));
    });
  });
});

import { Game, GameExecutable } from '@/types/types';
import { tryOnMounted, useAsyncState } from '@vueuse/core';
import { ref } from 'vue';
import { message } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { useGlobalState } from './app-state';

function isValidExecutable(item: any): item is GameExecutable {
    return (
        item !== null &&
        typeof item === 'object' &&
        typeof item.name === 'string' &&
        typeof item.os === 'string' &&
        typeof item.is_launcher === 'boolean'
    );
}

function isValidGame(item: any): item is Game {
    return (
        item !== null &&
        typeof item === 'object' &&
        typeof item.id === 'string' &&
        typeof item.name === 'string' &&
        Array.isArray(item.aliases) &&
        item.aliases.every((a: any) => typeof a === 'string') &&
        Array.isArray(item.executables) &&
        item.executables.every(isValidExecutable)
    );
}

function isValidGameList(data: any): data is Game[] {
    return Array.isArray(data) && data.every(isValidGame);
}

export function useFetchGameList() {
    const { addLog } = useGlobalState();

    async function fetchGameListGHMirror() {
        addLog('Fetching game list from GitHub mirror...');
        const response = await invoke('fetch_gamelist_gh_mirror');
        return response as Game[] | unknown[] | undefined;
    }

    async function fetchGameListFromDiscord() {
        addLog('Fetching game list directly from discord...');
        const response = await invoke('fetch_gamelist_from_discord');
        return response as Game[] | unknown[] | undefined;
    }

    const {
        state: gameListGHMirror,
        error: errorGH,
        isReady: isReadyGH,
        execute: executeGH,
        isLoading: isLoadingGH,
    } = useAsyncState<Game[] | unknown[] | undefined>(fetchGameListGHMirror, [], {
        immediate: false,
        resetOnExecute: true,
    });

    const {
        state: gameListFromDiscord,
        error: errorDiscord,
        isReady: isReadyDiscord,
        execute: executeDiscord,
        isLoading: isLoadingDiscord,
    } = useAsyncState(fetchGameListFromDiscord, [], {
        immediate: false,
        resetOnExecute: true,
    });

    const {
        state: bundledGameList,
        error: errorBundled,
        isReady: isReadyBundled,
        execute: executeBundled,
        isLoading: isLoadingBundled,
    } = useAsyncState(() => {
        addLog('Fetching bundled game list for fallback...');
        return import('../assets/gamelist.json').then((res) => res.default);
    }, [], {
        immediate: false,
        resetOnExecute: true,
    });

    const fetchError = ref<string | null>(null);
    const gameDB = ref<Game[]>([]);
    const allFetchDone = ref(false);

    async function fetchGameList() {
        allFetchDone.value = false;
        fetchError.value = null;
        gameDB.value = [];
        addLog('Fetching game list...');

        try {
            await executeGH();
            if (gameListGHMirror.value && gameListGHMirror.value.length > 0 && isValidGameList(gameListGHMirror.value)) {
                gameDB.value = gameListGHMirror.value as Game[];
                addLog(`Using game list from GitHub mirror. ${gameDB.value.length} entries.`);
                allFetchDone.value = true;
                return;
            }
            if (errorGH.value) {
                addLog('error', 'Error fetching game list from GitHub mirror.');
            } else {
                addLog('warning', 'GitHub mirror returned an invalid catalog; falling back to Discord.');
            }
        } catch (e) {
            addLog('error', 'Error fetching game list from GitHub mirror.');
        }

        try {
            await executeDiscord();
            if (gameListFromDiscord.value && gameListFromDiscord.value.length > 0 && isValidGameList(gameListFromDiscord.value)) {
                gameDB.value = gameListFromDiscord.value as Game[];
                addLog(`Using game list from Discord. ${gameDB.value.length} entries.`);
                allFetchDone.value = true;
                return;
            }
            if (errorDiscord.value) {
                addLog('error', 'Error fetching game list from Discord.');
            } else {
                addLog('warning', 'Discord returned an invalid catalog; falling back to bundled list.');
            }
        } catch (e) {
            addLog('error', 'Error fetching game list from Discord.');
        }

        try {
            await executeBundled();
            if (bundledGameList.value && bundledGameList.value.length > 0 && isValidGameList(bundledGameList.value)) {
                gameDB.value = bundledGameList.value as Game[];
                addLog(`Using bundled game list as fallback. ${gameDB.value.length} entries.`);
                allFetchDone.value = true;
                return;
            }
            if (errorBundled.value) {
                addLog('error', 'Error fetching bundled game list.');
            } else {
                addLog('warning', 'Bundled game list is empty or invalid.');
            }
        } catch (e) {
            addLog('error', 'Error fetching bundled game list.');
        }

        fetchError.value = 'Error fetching game list from all sources.';
        addLog('error', fetchError.value);
        await message('There was an error fetching the latest game list. ' + fetchError.value, {
            title: 'Game List Fetch Error',
            kind: 'error',
            buttons: { ok: 'OK' },
        });
        gameDB.value = [];
        allFetchDone.value = true;
    }

    tryOnMounted(async () => {
        await fetchGameList();
    });

    return {
        gameListGHMirror,
        gameListFromDiscord,
        bundledGameList,
        fetchError,
        isReadyGH,
        isReadyDiscord,
        isReadyBundled,
        gameDB,
        fetchGameList,
        isLoadingGH,
        isLoadingDiscord,
        isLoadingBundled,
        allFetchDone,
    };
}

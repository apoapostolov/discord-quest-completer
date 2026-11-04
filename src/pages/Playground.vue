<template>
    <div class="p-4 space-y-4 container mx-auto">
        <button class="bg-blue-500 hover:bg-blue-700 text-white font-bold py-2 px-4 rounded"
            :disabled="isChecking" @click="discordTest">{{ isChecking ? 'Checking...' : 'Check Discord IPC availability' }}</button>
        <p class="text-gray-700 dark:text-gray-300">
            {{ ipcAvailable === null ? 'Not checked or check failed.' : (ipcAvailable ? 'Discord IPC is available.' : 'Discord IPC is not available.') }}
            This read-only check does not connect an application or publish activity.
        </p>

        <!-- Logs Section -->
        <div class="mt-4 p-4 border rounded text-gray-700 dark:text-gray-300 dark:border-gray-600">
            <div class="flex items-center justify-between mb-2">
                <h2 class="text-lg font-semibold mb-2">Logs</h2>
                <button class="mt-2 font-bold py-1 px-3 rounded dark:bg-gray-700 bg-gray-300 hover:bg-gray-400 text-sm"
                    @click="clearLogs">Clear Logs</button>
            </div>

            <div class="max-h-64 overflow-y-auto p-2 rounded">
                <!-- <div v-if="logs.length === 0" class="text-gray-400">No logs available.</div>
                <ul v-else class="list-none">
                    <li v-for="(log, index) in logs" :key="index" class="text-sm text-gray-400">{{ log }}</li>
                </ul> -->
                <div v-if="logs.length === 0" class="text-gray-400">No logs available.</div>
                <ul v-else class="list-none">
                    <li v-for="(log, index) in logs" :key="index" class="text-sm">
                        <span class="text-gray-500">[{{ new Date(log.timestamp).toLocaleString() }}]</span>
                        <span :class="{
                            'text-blue-400': log.type === 'info',
                            'text-red-400': log.type === 'error',
                            'text-yellow-400': log.type === 'warning',
                            'text-green-400': log.type === 'debug'
                        }">
                            [{{ log.type.toUpperCase() }}]
                        </span>
                        <span class="ml-1">{{ log.message }}</span>
                    </li>
                </ul>
            </div>


        </div>
    </div>
</template>

<script setup lang="ts">
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useGlobalState } from '@/composables/app-state';

const ipcAvailable = ref<boolean | null>(null);
const isChecking = ref(false);
const { logs, addLog, clearLogs } = useGlobalState();

async function discordTest() {
    if (isChecking.value) return;
    isChecking.value = true;
    try {
        ipcAvailable.value = await invoke<boolean>('check_discord_connection');
        addLog('info', ipcAvailable.value ? 'Discord IPC is available.' : 'Discord IPC is not available.');
    } catch (error) {
        ipcAvailable.value = null;
        addLog('error', 'Discord IPC check failed: ' + String(error));
    } finally {
        isChecking.value = false;
    }
}


</script>

<style scoped></style>
<script setup lang="ts">
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import { ref, onMounted, onUnmounted } from 'vue';

interface AccentPayload {
  base_char: string;
  variants: string[];
}

const visible = ref(false);
const baseChar = ref('');
const variants = ref<string[]>([]);
let unlistenFn: (() => void) | null = null;

onMounted(async () => {
  unlistenFn = await listen<AccentPayload>('show-accent-menu', async (event) => {
    baseChar.value = event.payload.base_char;
    variants.value = event.payload.variants;
    visible.value = true;
    await getCurrentWindow().show();
    await getCurrentWindow().setFocus();
  });

  window.addEventListener('keydown', handleKeyDown);
});

onUnmounted(() => {
  unlistenFn?.();
  window.removeEventListener('keydown', handleKeyDown);
});

async function handleKeyDown(e: KeyboardEvent) {
  if (!visible.value) return;

  if (e.key === 'Escape') {
    await dismiss();
    return;
  }

  if (e.key === 'Enter' || e.key === ' ') {
    await select(0);
    return;
  }

  const num = parseInt(e.key);
  if (num >= 1 && num <= variants.value.length) {
    await select(num - 1);
  }
}

async function select(index: number) {
  visible.value = false;
  await invoke('select_accent', { index });
}

async function dismiss() {
  visible.value = false;
  await invoke('dismiss_accent');
}
</script>

<template>
  <div v-if="visible" class="accent-menu">
    <button
      v-for="(ch, i) in variants"
      :key="ch"
      class="accent-item"
      @click="select(i)"
    >
      <span class="accent-key">{{ i + 1 }}</span>
      <span class="accent-char">{{ ch }}</span>
    </button>
  </div>
</template>

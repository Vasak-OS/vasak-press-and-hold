<script setup lang="ts">
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useConfigStore } from '@vasakgroup/plugin-config-manager';
import { ref, onMounted, onUnmounted } from 'vue';

interface AccentPayload {
  base_char: string;
  variants: string[];
}

const visible = ref(false);
const baseChar = ref('');
const variants = ref<string[]>([]);
const unlisten: Array<() => void> = [];

// Esta página sólo dibuja. Mostrar y ocultar la ventana lo hace el demonio,
// que es quien decide cuándo abrirla; hacerlo desde acá dejaba la ventana en
// 0x0, sin mapear, y las variantes no se veían nunca.
//
// Tampoco recibe teclas, y es a propósito: si tomara el teclado, el acento
// elegido se escribiría acá adentro en vez de en la aplicación donde estabas
// escribiendo. Los números los atiende el demonio, que ya tiene el teclado
// tomado en exclusiva.
onMounted(async () => {
  unlisten.push(
    await listen<AccentPayload>('show-accent-menu', (event) => {
      baseChar.value = event.payload.base_char;
      variants.value = event.payload.variants;
      visible.value = true;
    }),
  );

  unlisten.push(
    await listen('hide-accent-menu', () => {
      visible.value = false;
      variants.value = [];
    }),
  );

  // Los colores, el radio y la tipografía salen de la configuración del
  // sistema, igual que en el resto de las aplicaciones. Va al final y sin
  // bloquear: si la configuración no se puede leer, el selector tiene que
  // aparecer igual, con los colores por defecto.
  try {
    const configStore = useConfigStore() as any;
    await configStore.loadConfig();
    unlisten.push(
      await listen('config-changed', () => {
        configStore.loadConfig();
      }),
    );
  } catch (error) {
    console.error('No se pudo leer la configuración de Vasak', error);
  }
});

onUnmounted(() => {
  unlisten.forEach((off) => off());
});

/** El clic sigue siendo una manera válida de elegir: el ratón sí llega acá. */
async function select(index: number) {
  visible.value = false;
  await invoke('select_accent', { index });
}
</script>

<template>
  <!-- La ventana es más grande que la tarjeta y transparente: la tarjeta se
       centra dentro, así queda centrada en pantalla sin importar cuántas
       variantes tenga la letra. -->
  <div class="flex h-screen w-screen items-center justify-center overflow-hidden">
    <div
      v-if="visible"
      class="flex w-max items-center gap-1 rounded-corner border border-ui-border bg-ui-bg/90 p-2 shadow-lg backdrop-blur-md"
    >
      <button
        v-for="(ch, i) in variants"
        :key="ch"
        class="flex h-11 w-11 shrink-0 flex-col items-center justify-center gap-0.5 rounded-corner transition-colors hover:bg-ui-surface"
        :title="`${baseChar} → ${ch}`"
        @click="select(i)"
      >
        <span class="text-xl leading-none text-tx-main">{{ ch }}</span>
        <span class="text-[10px] leading-none text-tx-muted">{{ i + 1 }}</span>
      </button>
    </div>
  </div>
</template>

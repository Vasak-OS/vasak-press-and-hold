<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useConfigStore } from '@vasakgroup/plugin-config-manager';
import { onMounted, onUnmounted, ref } from 'vue';

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
		})
	);

	unlisten.push(
		await listen('hide-accent-menu', () => {
			visible.value = false;
			variants.value = [];
		})
	);

	// Esta ventana se crea cuando el demonio la necesita, así que lo que hay que
	// mostrar puede haber llegado antes de que Vue montara: en ese caso está
	// anotado en el backend y no en un evento que ya pasó. Va después de
	// suscribirse, o habría un hueco entre reclamar y escuchar.
	//
	// Lo normal es que devuelva nada: la ventana se suele crear por el
	// calentamiento del keydown, y esa tecla muchas veces termina siendo un tap.
	try {
		const pendiente = await invoke<AccentPayload | null>('picker_ready');
		if (pendiente) {
			baseChar.value = pendiente.base_char;
			variants.value = pendiente.variants;
			visible.value = true;
		}
	} catch (error) {
		console.error('No se pudo reclamar el acento pendiente', error);
	}

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
			})
		);
	} catch (error) {
		console.error('No se pudo leer la configuración de Vasak', error);
	}
});

onUnmounted(() => {
	unlisten.forEach((off) => {
		off();
	});
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

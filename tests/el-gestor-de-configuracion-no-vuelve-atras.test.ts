import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/**
 * `vasak.conf` es de todo el escritorio, y esta aplicación es una de las que lo
 * abre. El gestor de configuración anterior a la 2.6.0 no devolvía el archivo:
 * devolvía su modelo reserializado, y **borraba toda clave que el modelo no
 * conociera** —la disposición de los widgets del escritorio, la pausa del fondo
 * en vídeo—. Como el archivo es compartido, alcanzaba con que **una** aplicación
 * atrasada lo leyera y lo escribiera para dejar sin configuración a las demás.
 *
 * El arreglo entró en la 2.6.0 (`8ee9c00`): cada sección del modelo lleva un
 * `serde(flatten)` con lo que no conoce, así que lo desconocido entra, sale y
 * vuelve al archivo igual que estaba.
 *
 * Acá los dos manifiestos van a la **2.7.0**, que es lo que los demás no pueden:
 * la 2.7.0 pide `pinia ^4` y esta aplicación es una de las dos que ya está en
 * pinia 4. Por eso no hay nota en `vasak.bibliotecasAtrasadas` ni rango fijado
 * con `~`.
 *
 * Se mira el **bloqueo** y no el rango, porque el rango es el que admite volver
 * atrás sin que nada falle ni avise.
 */

const LA_VERSION_QUE_ARREGLA = [2, 6, 0] as const;

const raiz = join(import.meta.dir, '..');

function comparar(version: string): number {
	const partes = version.split('.').map(Number);

	for (const [i, minima] of LA_VERSION_QUE_ARREGLA.entries()) {
		const parte = partes[i] ?? 0;
		if (parte !== minima) return parte - minima;
	}

	return 0;
}

function versionEnCargo(): string {
	const candado = readFileSync(join(raiz, 'src-tauri', 'Cargo.lock'), 'utf8');
	// Los paquetes van en bloques `[[package]]`; hay que leer la versión del
	// bloque de este nombre y no la del siguiente que aparezca en el archivo.
	const bloque = candado
		.split('[[package]]')
		.find((b) => /^\s*name = "tauri-plugin-config-manager"\s*$/m.test(b));

	expect(bloque, 'el gestor no está en Cargo.lock').toBeDefined();

	const version = bloque?.match(/^\s*version = "([^"]+)"\s*$/m)?.[1];
	expect(version, 'el gestor no declara versión en Cargo.lock').toBeDefined();

	return version as string;
}

function versionEnBun(): string {
	const candado = readFileSync(join(raiz, 'bun.lock'), 'utf8');
	const version = candado.match(
		/"@vasakgroup\/plugin-config-manager@(\d[^"]*)"/,
	)?.[1];

	expect(version, 'el gestor no está en bun.lock').toBeDefined();

	return version as string;
}

describe('el gestor de configuración que se instala', () => {
	test('el de Rust no es anterior a la 2.6.0, que es donde dejó de comerse las claves ajenas', () => {
		expect(comparar(versionEnCargo())).toBeGreaterThanOrEqual(0);
	});

	test('y el de JavaScript tampoco', () => {
		expect(comparar(versionEnBun())).toBeGreaterThanOrEqual(0);
	});

	test('la comparación distingue una 2.5.x de una 2.6.0, que es lo que se está cuidando', () => {
		// Sin esto las dos de arriba pasan aunque `comparar` devuelva siempre 0.
		expect(comparar('2.5.0')).toBeLessThan(0);
		expect(comparar('2.5.9')).toBeLessThan(0);
		expect(comparar('2.6.0')).toBe(0);
		expect(comparar('2.7.0')).toBeGreaterThan(0);
		expect(comparar('3.0.0')).toBeGreaterThan(0);
	});
});

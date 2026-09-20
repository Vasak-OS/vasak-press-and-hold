/**
 * Que ningún archivo del repositorio aplane los tipos de los componentes.
 *
 * Había un `declare module '*.vue'` en `vite-env.d.ts`, con
 * `DefineComponent<object, object, any>`: herencia de cuando el chequeo lo
 * hacía `tsc` a secas. `vue-tsc` entiende los `.vue` de forma nativa y no lo
 * necesita.
 *
 * **Lo que hacía hoy: nada.** Es una declaración *ambiente*, y esas se aplican
 * sólo cuando TypeScript no encuentra un `.d.ts` —o el `.vue` de verdad— para
 * el import. Se comprobó metiendo un error de tipo a propósito con el comodín
 * puesto y sin él: salta en los dos casos.
 *
 * Se saca igual porque es una trampa armada: en cuanto un import no resuelva a
 * un archivo con tipos, el comodín se aplica y el chequeo deja de mirar sin que
 * nada avise. Eso ya pasó en vasak-desktop, con los componentes de
 * `vue-libvasak` en su línea 0.2.
 */

import { describe, expect, test } from 'bun:test';
import { fileURLToPath } from 'node:url';

// `fileURLToPath` y no `.pathname`: éste deja los caracteres escapados, así que
// un checkout en una ruta con espacios mandaría a `Bun.Glob` y `Bun.file` a una
// carpeta que no existe.
const raiz = fileURLToPath(new URL('..', import.meta.url));
/** Este mismo archivo, relativo a la raíz. */
const propio = fileURLToPath(import.meta.url).slice(raiz.length);

/**
 * Todo lo que el chequeo de tipos mira, no sólo `src`.
 *
 * El `tsconfig` de la raíz incluye también `tests/**` y los `.tsx`, y el de
 * node los archivos de configuración sueltos. Una declaración puesta en
 * cualquiera de esos lugares aplana los tipos igual, y con un patrón más
 * angosto las pruebas de abajo pasarían sin haberla visto. Lo marcó la
 * revisión.
 */
const fuentes = (
	await Promise.all(
		['src/**/*.{ts,tsx,mts,cts,vue}', 'tests/**/*.{ts,tsx,vue}', '*.{ts,mts,cts}'].map(
			async (patron) => await Array.fromAsync(new Bun.Glob(patron).scan({ cwd: raiz }))
		)
	)
).flat()
	// Menos este archivo. Los patrones que busca los lleva escritos adentro,
	// así que al ampliar el escaneo a `tests/` empezó a encontrarse a sí mismo.
	.filter((ruta) => ruta !== propio);

async function conteniendo(patron: RegExp): Promise<string[]> {
	const hallados: string[] = [];
	for (const ruta of fuentes) {
		if (patron.test(await Bun.file(`${raiz}${ruta}`).text())) hallados.push(ruta);
	}
	return hallados.sort();
}

describe('los tipos de los componentes', () => {
	test('y las dos pruebas que siguen miran archivos de verdad', () => {
		// Las dos buscan algo que no tiene que aparecer, así que pasan solas si
		// la lista viene vacía —una `raiz` mal armada y no hay nada que mirar—.
		expect(fuentes).toContain('src/vite-env.d.ts');
		expect(fuentes).toContain('src/main.ts');
		// Y que los tres patrones traigan algo: el de `tests` y el de la raíz se
		// sumaron porque el de `src` solo dejaba huecos, y un patrón que no
		// encuentra nada los deja igual.
		expect(fuentes.some((ruta) => ruta.startsWith('tests/'))).toBe(true);
		expect(fuentes).toContain('vite.config.ts');
		expect(fuentes.length).toBeGreaterThan(3);
	});

	test('no los aplana ningún comodín de .vue', async () => {
		expect(await conteniendo(/declare\s+module\s+['"]\*\.vue['"]/)).toEqual([]);
	});

	test('ni los redeclara a mano ningún paquete del ecosistema', async () => {
		// Un `declare module` de un paquete instalado gana siempre, así que lo
		// que diga ese archivo es lo único que se comprueba. Pasó con
		// `@vasakgroup/vue-libvasak` en la galería y en el escritorio.
		expect(await conteniendo(/declare\s+module\s+['"]@vasakgroup\//)).toEqual([]);
	});
});

/**
 * El `tsconfig` sin sus comentarios, listo para `JSON.parse`.
 *
 * Es JSONC, así que `.json()` se cae con «Unrecognized token '/'». Y quitar los
 * comentarios con una expresión regular tampoco alcanza: el alias `"@/*"` lleva
 * un `/*` adentro de las comillas, y el quitador se come desde ahí hasta el
 * próximo `*​/`, dejando el JSON partido. Por eso este recorre el texto sabiendo
 * cuándo está dentro de una cadena.
 */
async function leerTsconfig(): Promise<{
	vueCompilerOptions?: { strictTemplates?: boolean };
}> {
	const crudo = await Bun.file(`${raiz}tsconfig.json`).text();
	let salida = '';
	let enCadena = false;
	let escapado = false;
	for (let i = 0; i < crudo.length; i++) {
		const caracter = crudo[i];
		if (enCadena) {
			salida += caracter;
			if (escapado) escapado = false;
			else if (caracter === '\\') escapado = true;
			else if (caracter === '"') enCadena = false;
			continue;
		}
		if (caracter === '"') {
			enCadena = true;
			salida += caracter;
			continue;
		}
		if (caracter === '/' && crudo[i + 1] === '/') {
			while (i < crudo.length && crudo[i] !== '\n') i++;
			salida += '\n';
			continue;
		}
		if (caracter === '/' && crudo[i + 1] === '*') {
			i += 2;
			while (i < crudo.length && !(crudo[i] === '*' && crudo[i + 1] === '/')) i++;
			i++;
			continue;
		}
		salida += caracter;
	}
	return JSON.parse(salida);
}

describe('el chequeo de las plantillas', () => {
	test('mira cada atributo, no sólo los que reconoce', async () => {
		// Sin `strictTemplates`, `vue-tsc` comprueba el tipo de las propiedades
		// que **sí** existen y no dice nada de una que no existe, de un evento
		// que el componente no emite, ni de un atributo inventado sobre un
		// elemento. Un `@click` sobre un componente sin `defineEmits` funciona
		// por caída de atributos y nunca se nota; un `:size` sobre un `<img>` no
		// hace nada y tampoco.
		const tsconfig = await leerTsconfig();

		expect(tsconfig.vueCompilerOptions?.strictTemplates).toBe(true);
	});

	test('y los `data-*` siguen permitidos, que es la excepción legítima', async () => {
		// HTML los permite todos, y acá marcan nodos que después se buscan con
		// `closest()` o `querySelector()`. Declararlos uno por uno deja la lista
		// vieja en cuanto alguien marca un nodo nuevo, así que se declara la
		// forma.
		const declaracion = await Bun.file(`${raiz}src/tipos-de-plantilla.d.ts`).text();

		// Con una expresión regular y no con `toContain`: escrito como cadena,
		// `'data-${string}'` hace que el linter avise de un marcador de
		// plantilla que no se interpola, y no es eso lo que pasa.
		expect(declaracion).toMatch(/\[atributo: `data-\$\{string\}`\]/);
		expect(declaracion).toContain("declare module 'vue'");
	});
});

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
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

/** Un `declare module '*.vue'`, con cualquiera de las dos comillas. */
const COMODIN = /declare\s+module\s+['"]\*\.vue['"]/;
/** Un `declare module` a mano de un paquete del ecosistema. */
const PAQUETE = /declare\s+module\s+['"]@vasakgroup\//;

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
async function escanear(cwd: string): Promise<string[]> {
	const listas = await Promise.all(
		['src/**/*.{ts,tsx,mts,cts,vue}', 'tests/**/*.{ts,tsx,vue}', '*.{ts,mts,cts}'].map(
			async (patron) => await Array.fromAsync(new Bun.Glob(patron).scan({ cwd })),
		),
	);
	return listas.flat();
}

const fuentes = (await escanear(raiz))
	// Menos este archivo. Los patrones que busca los lleva escritos adentro,
	// así que al ampliar el escaneo a `tests/` empezó a encontrarse a sí mismo.
	.filter((ruta) => ruta !== propio);

async function conteniendo(
	patron: RegExp,
	archivos: string[] = fuentes,
	base: string = raiz,
): Promise<string[]> {
	const hallados: string[] = [];
	for (const ruta of archivos) {
		if (patron.test(await Bun.file(`${base}${ruta}`).text())) hallados.push(ruta);
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

	test('y los dos patrones encuentran lo que buscan', async () => {
		// El caso positivo, sobre un repositorio de mentira armado aparte. Sin
		// esto, un patrón que dejó de matchear —o un `conteniendo` que siempre
		// devuelve `[]`— deja las dos pruebas de ausencia en verde sin haber
		// mirado nada. No es hipotético: vasak-session-manager se salvó del
		// barrido que sacó el comodín justo así, porque el patrón buscaba
		// comillas simples y ahí estaba escrito con dobles.
		const falso = `${tmpdir()}/vsk-guardia-${Bun.randomUUIDv7()}/`;
		try {
			// Las dos comillas en el comodín, una en cada archivo.
			await Bun.write(`${falso}src/dobles.d.ts`, 'declare module "*.vue" {}\n');
			await Bun.write(`${falso}src/simples.d.ts`, "declare module '*.vue' {}\n");
			await Bun.write(
				`${falso}src/paquete.d.ts`,
				"declare module '@vasakgroup/vue-libvasak' {}\n",
			);
			await Bun.write(`${falso}src/inocente.ts`, 'export const nada = 1;\n');

			const archivos = await escanear(falso);
			expect(archivos).toHaveLength(4);

			expect(await conteniendo(COMODIN, archivos, falso)).toEqual([
				'src/dobles.d.ts',
				'src/simples.d.ts',
			]);
			expect(await conteniendo(PAQUETE, archivos, falso)).toEqual(['src/paquete.d.ts']);
		} finally {
			await Bun.$`rm -rf ${falso}`.quiet();
		}
	});

	test('no los aplana ningún comodín de .vue', async () => {
		expect(await conteniendo(COMODIN)).toEqual([]);
	});

	test('ni los redeclara a mano ningún paquete del ecosistema', async () => {
		// Un `declare module` de un paquete instalado gana siempre, así que lo
		// que diga ese archivo es lo único que se comprueba. Pasó con
		// `@vasakgroup/vue-libvasak` en la galería y en el escritorio.
		expect(await conteniendo(PAQUETE)).toEqual([]);
	});
});

/**
 * El `tsconfig` sin sus comentarios.
 *
 * Es JSONC, así que `.json()` se cae con «Unrecognized token '/'». Y quitar los
 * comentarios con una expresión regular tampoco alcanza: el alias `"@/*"` lleva
 * un `/*` adentro de las comillas, y el quitador se come desde ahí hasta el
 * próximo cierre, dejando el JSON partido. Por eso éste recorre el texto
 * sabiendo cuándo está dentro de una cadena.
 */
function sinComentarios(crudo: string): string {
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
	return salida;
}

/**
 * Y sin sus comas finales, que JSONC permite y `JSON.parse` no.
 *
 * Un `"strict": true,` antes de la llave de cierre es un `tsconfig` válido —y
 * lo que escribe medio editor al reordenar—, pero tumba el `JSON.parse` y con
 * él toda esta suite. Lo marcó la revisión. Va aparte del quitador de
 * comentarios para poder mirar la coma y la llave sin un comentario en el
 * medio.
 */
function sinComasFinales(texto: string): string {
	let salida = '';
	let enCadena = false;
	let escapado = false;
	for (let i = 0; i < texto.length; i++) {
		const caracter = texto[i];
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
		if (caracter === ',') {
			let j = i + 1;
			while (j < texto.length && /\s/.test(texto[j])) j++;
			if (texto[j] === '}' || texto[j] === ']') continue;
		}
		salida += caracter;
	}
	return salida;
}

/** Corre el chequeo sobre un proyecto suelto y devuelve su salida. */
async function chequear(tsconfig: string): Promise<{ codigo: number; salida: string }> {
	const proceso = Bun.spawn(['bunx', 'vue-tsc', '--noEmit', '-p', tsconfig], {
		cwd: raiz,
		stdout: 'pipe',
		stderr: 'pipe',
	});
	const salida = `${await new Response(proceso.stdout).text()}${await new Response(proceso.stderr).text()}`;
	return { codigo: await proceso.exited, salida };
}

async function leerTsconfig(): Promise<{
	vueCompilerOptions?: { strictTemplates?: boolean };
}> {
	const crudo = await Bun.file(`${raiz}tsconfig.json`).text();
	return JSON.parse(sinComasFinales(sinComentarios(crudo)));
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

	test('y el `tsconfig` se lee aunque lleve comentarios, alias y comas finales', () => {
		// Las tres cosas son JSONC válido y las tres rompen `JSON.parse`. El
		// alias es el caso que tumbó al primer quitador: el `/*` va adentro de
		// las comillas.
		const crudo = `{
			// una línea
			"vueCompilerOptions": { "strictTemplates": true },
			/* y un bloque */
			"compilerOptions": {
				"paths": { "@/*": ["./src/*"] },
				"lib": ["ES2024", "DOM"],
			},
		}`;

		const leido = JSON.parse(sinComasFinales(sinComentarios(crudo)));

		expect(leido.vueCompilerOptions.strictTemplates).toBe(true);
		expect(leido.compilerOptions.paths['@/*']).toEqual(['./src/*']);
		expect(leido.compilerOptions.lib).toEqual(['ES2024', 'DOM']);
	});

	test('y los `data-*` siguen permitidos: se comprueba compilando', async () => {
		// HTML los permite todos, y acá marcan nodos que después se buscan con
		// `closest()` o `querySelector()`.
		//
		// Se compila un `.vue` con un `data-*` inventado en vez de buscar el
		// texto de la declaración: buscarlo pasa igual si el texto quedó en un
		// comentario o en otra interfaz, y no dice nada de si la declaración
		// llega a aplicarse. Lo marcó la revisión.
		const { codigo, salida } = await chequear('tests/fixtures/tsconfig-data.json');

		expect(salida).toBe('');
		expect(codigo).toBe(0);
	}, 120_000);

	test('y sin la declaración, ese mismo `data-*` falla', async () => {
		// La otra mitad: sin esto, lo de arriba pasaría también si
		// `strictTemplates` no llegara al fixture, o si Vue permitiera los
		// `data-*` por su cuenta. Se compila el mismo archivo con el mismo
		// `tsconfig` menos la declaración.
		const tsconfig = JSON.parse(
			await Bun.file(`${raiz}tests/fixtures/tsconfig-data.json`).text()
		) as { include: string[] };
		const sinDeclaracion = tsconfig.include.filter((ruta) => !ruta.endsWith('.d.ts'));
		expect(sinDeclaracion).toHaveLength(tsconfig.include.length - 1);

		const carpeta = `${raiz}tests/fixtures/.sin-declaracion-${Bun.randomUUIDv7()}`;
		try {
			await Bun.write(
				`${carpeta}/tsconfig.json`,
				JSON.stringify({ ...tsconfig, include: sinDeclaracion })
			);
			await Bun.write(
				`${carpeta}/UsoDeDataAttr.vue`,
				await Bun.file(`${raiz}tests/fixtures/UsoDeDataAttr.vue`).text()
			);

			const { codigo, salida } = await chequear(
				`tests/fixtures/${carpeta.split('/').pop()}/tsconfig.json`
			);

			// Con posición y sobre el archivo: un `tsconfig` que no mire nada
			// también termina con error —«No inputs were found»— y daría por
			// buena la prueba sin haber compilado el `data-*`.
			expect(salida).toMatch(/UsoDeDataAttr\.vue\(\d+,\d+\): error TS\d+/);
			expect(codigo).not.toBe(0);
		} finally {
			await Bun.$`rm -rf ${carpeta}`.quiet();
		}
	}, 120_000);
});

import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from '@/App.vue';
import '@/assets/main.css';

// Una violación de CSP no se ve: el recurso simplemente no carga y la interfaz
// queda a medias sin decir nada. Esto la manda a la consola, que es donde se
// puede encontrar al ajustar la política.
document.addEventListener('securitypolicyviolation', (evento) => {
	// Sin la query ni el fragmento: `blockedURI` puede llevar tokens o
	// identificadores. Para saber qué directiva falló alcanza el origen y la ruta.
	let recurso = evento.blockedURI || '(en línea)';
	try {
		const url = new URL(recurso);
		recurso = url.protocol === 'data:' ? 'data:(recortado)' : `${url.origin}${url.pathname}`;
	} catch {
		// No era una URL absoluta —'inline', 'eval', una ruta relativa—: va tal cual.
	}
	console.error(
		`[CSP] bloqueado ${recurso} por la directiva ` +
			`«${evento.violatedDirective}» en ${evento.sourceFile ?? 'documento'}:${evento.lineNumber}`
	);
});

// El selector se dibuja sobre lo que estés escribiendo, así que arranca en
// oscuro: la configuración del sistema lo corrige apenas se lee.
document.documentElement.classList.add('dark');

createApp(App).use(createPinia()).mount('#app');

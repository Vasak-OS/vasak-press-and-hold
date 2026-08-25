import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from '@/App.vue';
import '@/assets/main.css';

// Una violación de CSP no se ve: el recurso simplemente no carga y la interfaz
// queda a medias sin decir nada. Esto la manda a la consola, que es donde se
// puede encontrar al ajustar la política.
document.addEventListener('securitypolicyviolation', (evento) => {
	console.error(
		`[CSP] bloqueado ${evento.blockedURI || '(en línea)'} por la directiva ` +
			`«${evento.violatedDirective}» en ${evento.sourceFile ?? 'documento'}:${evento.lineNumber}`
	);
});

// El selector se dibuja sobre lo que estés escribiendo, así que arranca en
// oscuro: la configuración del sistema lo corrige apenas se lee.
document.documentElement.classList.add('dark');

createApp(App).use(createPinia()).mount('#app');

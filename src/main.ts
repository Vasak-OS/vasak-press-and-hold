import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from '@/App.vue';
import '@/assets/main.css';

// El selector se dibuja sobre lo que estés escribiendo, así que arranca en
// oscuro: la configuración del sistema lo corrige apenas se lee.
document.documentElement.classList.add('dark');

createApp(App).use(createPinia()).mount('#app');

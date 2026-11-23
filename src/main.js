import { createApp } from 'vue'
import { ElLoading } from 'element-plus'
import 'element-plus/dist/index.css'
import App from './App.vue'
import './style.css'
import './management.css'

createApp(App).use(ElLoading).mount('#app')

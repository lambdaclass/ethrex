import fs from 'fs'
import path from 'path'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import basicSsl from '@vitejs/plugin-basic-ssl'

// Use real Tailscale certs if available, otherwise fall back to self-signed
const tlsCert = path.resolve(process.env.HOME || '~', 'tls.crt')
const tlsKey = path.resolve(process.env.HOME || '~', 'tls.key')
const hasRealCert = fs.existsSync(tlsCert) && fs.existsSync(tlsKey)

export default defineConfig({
  plugins: [react(), tailwindcss(), ...(!hasRealCert ? [basicSsl()] : [])],
  server: {
    host: '0.0.0.0',
    allowedHosts: true,
    ...(hasRealCert && {
      https: {
        cert: fs.readFileSync(tlsCert),
        key: fs.readFileSync(tlsKey),
      },
    }),
    proxy: {
      '/api': 'http://localhost:3000',
      '/rpc': {
        target: 'http://localhost:8545',
        rewrite: (path) => path.replace(/^\/rpc/, ''),
      },
    },
  },
})

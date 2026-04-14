/*
|--------------------------------------------------------------------------
| Routes file
|--------------------------------------------------------------------------
|
| The routes file is used for defining the HTTP routes.
|
*/
import { glob } from 'node:fs/promises'
import { fileURLToPath, pathToFileURL } from 'node:url'
import path from 'node:path'

import router from '@adonisjs/core/services/router'
import openapi from '@foadonis/openapi/services/main'

openapi.registerRoutes()
router.get('/', () => {
  return { hello: 'world' }
})

// Resolves '#app/placeholder' → 'file:///project/app/placeholder.js'
// then dirname strips the filename → '/project/app'
const appDir = path.dirname(fileURLToPath(import.meta.resolve('#app/placeholder')))

/**
 * Detect whether we're running TypeScript source (dev, via AdonisJS JIT loader)
 * or compiled JavaScript (production build). import.meta.url retains the .ts
 * extension in dev because AdonisJS's loader hook preserves it.
 */
const ext = import.meta.url.split("?")[0].endsWith('.ts') ? 'ts' : 'js'

for await (const routeFile of glob(`**/routes.${ext}`, { cwd: appDir })) {
  await import(pathToFileURL(path.join(appDir, routeFile)).href)
}

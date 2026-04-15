/*
|--------------------------------------------------------------------------
| Routes file
|--------------------------------------------------------------------------
|
| The routes file is used for defining the HTTP routes.
|
*/

import router from '@adonisjs/core/services/router'
import { controllers } from '#generated/controllers'

router
  .group(() => {
    router
      .group(() => {
        router
          .group(() => {
            router.get('/redirect', [controllers.auth.GoogleOauth, 'googleRedirect'])
            router.get('/callback', [controllers.auth.GoogleOauth, 'googleCallback'])
          })
          .prefix('google')
      })
      .prefix('auth')
  })
  .prefix('/v2')

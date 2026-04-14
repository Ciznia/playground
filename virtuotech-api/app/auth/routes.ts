/*
|--------------------------------------------------------------------------
| Routes file
|--------------------------------------------------------------------------
|
| The routes file is used for defining the HTTP routes.
|
*/

import { middleware } from '#start/kernel'
import router from '@adonisjs/core/services/router'
import { controllers } from '#generated/controllers'

router
  .group(() => {
    router
      .group(() => {
        router.group(() => {
          router.get('/redirect', [controllers.auth.GoogleOauth, 'googleRedirect'])
          router.get('/callback', [controllers.auth.GoogleOauth, 'googleCallback'])
        })
        .prefix('google')

        router.post('signup', [controllers.auth.NewAccount, 'store'])
        router.post('login', [controllers.auth.AccessToken, 'store'])
        router.post('logout', [controllers.auth.AccessToken, 'destroy']).use(middleware.auth())
      })
      .prefix('auth')
      .as('auth')

    router
      .group(() => {
        router.get('/profile', [controllers.auth.Profile, 'show'])
      })
      .prefix('account')
      .as('profile')
      .use(middleware.auth())
  })
  .prefix('/v2')

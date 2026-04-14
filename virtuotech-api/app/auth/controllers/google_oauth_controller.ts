import type { HttpContext } from '@adonisjs/core/http'

export default class GoogleOauthController {

  async googleRedirect({ ally, session }: HttpContext) {
    session.put('redirect.previousUrl', '/v2/auth/login')
    return ally.use('google').redirect((request) => {
      request.param('access_type', 'offline')
      request.param('prompt', 'consent')
    })
  }

  async googleCallback({ ally }: HttpContext) {
    const google = ally.use('google')
    const googleUser = await google.user()
    return googleUser
  }
}

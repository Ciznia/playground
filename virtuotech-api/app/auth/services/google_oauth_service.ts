import UserTransformer from '#app/auth/transformers/user_transformer'
import User from '#app/auth/models/user'
import type { AllyService } from '@adonisjs/ally/types'
import type { Session } from '@adonisjs/session'

export default class GoogleOauthService {
  static async googleRedirect(ally: AllyService, session: Session) {
    session.put('redirect.previousUrl', '/v2/auth/login')
    return ally.use('google').redirect((request) => {
      request.param('access_type', 'offline')
      request.param('prompt', 'consent')
    })
  }

  static async googleCallback(ally: AllyService) {
    const google = ally.use('google')
    const googleUser = await google.user()
    const user = await User.firstOrCreate(
      { email: googleUser.email },
      {
        email: googleUser.email,
        givenName: googleUser.original.given_name,
        familyName: googleUser.original.family_name,
      }
    )

    return UserTransformer.transform(user)
  }
}

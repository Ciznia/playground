import type { HttpContext } from '@adonisjs/core/http'
import { ApiOperation, ApiResponse } from '@foadonis/openapi/decorators'
import UserTransformer from '#app/auth/transformers/user_transformer'
import User from '#app/auth/models/user'

export default class GoogleOauthController {
  @ApiOperation({
    summary: 'Redirects the user to Google for authentication',
    tags: ['Authentication'],
  })
  @ApiResponse({
    status: 302,
    description: 'Redirects to Google OAuth consent screen',
  })
  async googleRedirect({ ally, session }: HttpContext) {
    session.put('redirect.previousUrl', '/v2/auth/login')
    return ally.use('google').redirect((request) => {
      request.param('access_type', 'offline')
      request.param('prompt', 'consent')
    })
  }

  @ApiOperation({
    summary: 'Handles the Google OAuth callback',
    description: 'Retrieves the user information and create a new user if it does not exist',
    tags: ['Authentication'],
  })
  @ApiResponse({
    status: 200,
    description: 'User authenticated successfully',
  })
  async googleCallback({ ally }: HttpContext) {
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
